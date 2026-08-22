use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use tokio::sync::broadcast::Sender;
use serde_json::json;


use crate::{
    domain::errors::LegacyAppError,

    domain::{
        DesiredConfig, HealthState, LastOperation, Lifecycle, PublicOperation, PublicState,
        WifiPublicState,
    },
    infrastructure::backend::RouterBackend,
    infrastructure::storage::StateStore,
};

pub mod handlers;
mod maintenance;
mod operations;
mod reconcile;
mod recovery;
mod wan;

use crate::application::state::{generate_id, snapshot};

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub reconcile_interval: Duration,
    pub verify_timeout: Duration,
    pub verify_sample_delay: Duration,
    pub rollback_verify_timeout: Duration,
    pub rpcd_rollback_timeout_secs: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            reconcile_interval: Duration::from_secs(2),
            verify_timeout: Duration::from_secs(90),
            verify_sample_delay: Duration::from_millis(400),
            rollback_verify_timeout: Duration::from_secs(10),
            rpcd_rollback_timeout_secs: 120,
        }
    }
}

pub(crate) struct Inner {
    pub config: DesiredConfig,
    pub lifecycle: Lifecycle,
    pub maintenance: bool,
    pub maintenance_exiting: bool,
    pub maintenance_reason: Option<String>,
    pub observed_configs: BTreeMap<String, crate::domain::WifiNetworkConfig>,
    pub runtime_healthy: bool,
    pub wan: crate::domain::WanPublicState,
    pub active_operation: Option<PublicOperation>,
    pub last_user_operation: Option<LastOperation>,
    pub last_system_error: Option<LegacyAppError>,
    pub event_seq: u64,
    pub boot_id: String,
    pub health: HealthState,
    pub repair_failures: u8,
    pub traffic: crate::domain::traffic::TrafficState,
    pub ddns_status: crate::domain::DdnsStatus,
    pub extender_ports: std::collections::HashMap<String, Vec<crate::domain::ports::PhysicalPort>>,
    pub extender_clients: std::collections::HashMap<String, Vec<crate::domain::extender::ExtenderClient>>,
    pub latest_scans: std::collections::HashMap<String, Vec<crate::domain::extender::ScannedNetwork>>,
    pub pending_extenders: Vec<crate::domain::extender::PendingExtender>,
    pub extender_pairing_status: String,
}

pub struct App {
    pub(crate) backend: Arc<dyn RouterBackend>,
    pub(crate) store: StateStore,
    pub(crate) inner: Mutex<Inner>,
    pub(crate) event_tx: Sender<PublicState>,
    pub(crate) shutdown: AtomicBool,
    pub(crate) op_counter: AtomicUsize,
    pub(crate) timing: Timing,
    pub subscriptions: crate::application::subscription::SubscriptionManager,
    pub rrm_tx: Sender<()>,
}

impl App {
    pub fn bootstrap(
        backend: Arc<dyn RouterBackend>,
        store: StateStore,
        event_tx: Sender<PublicState>,
    ) -> Arc<Self> {
        Self::bootstrap_with_timing(backend, store, event_tx, Timing::default())
    }

    pub fn bootstrap_with_timing(
        backend: Arc<dyn RouterBackend>,
        store: StateStore,
        event_tx: Sender<PublicState>,
        timing: Timing,
    ) -> Arc<Self> {
        let store_ready = store.ensure();
        let store_err = store_ready.as_ref().err().cloned();
        let (config, lifecycle, init_error) = config_init::load_initial_config(backend.as_ref(), &store);
        let startup_error = store_err.or(init_error);

        let boot_id = generate_id("boot");
        let wan_status = backend.read_wan_runtime_status().unwrap_or_default();
        let initial_core_health = if lifecycle == Lifecycle::Degraded {
            "error".into()
        } else {
            "ok".into()
        };

        let app = Arc::new(Self {
            backend,
            store,
            inner: Mutex::new(Inner {
                config,
                lifecycle,
                maintenance: false,
                maintenance_exiting: false,
                maintenance_reason: None,
                observed_configs: BTreeMap::new(),
                runtime_healthy: false,
                wan: wan_status,
                active_operation: None,
                last_user_operation: None,
                last_system_error: startup_error,
                event_seq: 0,
                boot_id,
                health: HealthState {
                    core: initial_core_health,
                    ubus: "ok".into(),
                    rpcd: "ok".into(),
                    wireless: "unknown".into(),
                    wan: "ok".into(),
                },
                repair_failures: 0,
                traffic: crate::domain::traffic::TrafficState::default(),
                ddns_status: crate::domain::DdnsStatus::default(),
                extender_ports: std::collections::HashMap::new(),
                extender_clients: std::collections::HashMap::new(),
                latest_scans: std::collections::HashMap::new(),
                pending_extenders: Vec::new(),
                extender_pairing_status: "idle".to_string(),
            }),
            event_tx,
            shutdown: AtomicBool::new(false),
            op_counter: AtomicUsize::new(1),
            timing,
            subscriptions: crate::application::subscription::SubscriptionManager::new(),
            rrm_tx: tokio::sync::broadcast::channel(16).0,
        });

        app.init();
        app
    }

    fn init(self: &Arc<Self>) {
        if let Ok(Some(journal)) = self.store.load_transaction() {
            if let Err(error) = self.recover_from_journal(&journal) {
                self.mark_degraded(error);
            }
        } else {
            self.refresh_observed();
        }

        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if inner.lifecycle == Lifecycle::Booting {
                inner.lifecycle = Lifecycle::Ready;
            }
        }
        self.publish();
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    pub fn state(&self) -> PublicState {
        let inner = self.inner.lock().expect("app state poisoned");
        snapshot(&inner)
    }

    pub fn wifi_get(&self) -> WifiPublicState {
        self.state().wifi
    }

    pub fn last_or_active_operation(&self) -> serde_json::Value {
        let state = self.state();
        json!({
            "active": state.active_operation,
            "last": state.last_user_operation
        })
    }

    pub fn health(&self) -> HealthState {
        self.state().health
    }

    pub fn ports_list(&self) -> Result<Vec<crate::domain::ports::PhysicalPort>, LegacyAppError> {
        self.backend.ports_list()
    }

    pub fn system_info(&self) -> crate::domain::system::SystemInfo {
        self.backend.read_system_info().unwrap_or_default()
    }

    pub fn devices_list(&self) -> Result<Vec<crate::domain::device::Device>, LegacyAppError> {
        let (extenders, extender_clients) = {
            let inner = self.inner.lock().unwrap();
            (inner.config.extenders.clone(), inner.extender_clients.clone())
        };
        self.backend.read_devices(&extenders, &extender_clients)
    }

    pub fn has_active_subscribers(&self) -> bool {
        self.subscriptions.has_active_subscribers()
    }

    pub(crate) fn mesh_add_pending(&self, extender: crate::domain::extender::PendingExtender) {
        {
            let mut inner = self.inner.lock().unwrap();
            if !inner.pending_extenders.iter().any(|e| e.mac == extender.mac) {
                if inner.pending_extenders.len() >= 50 {
                    inner.pending_extenders.remove(0);
                }
                inner.pending_extenders.push(extender);
            }
        }
        self.publish();
    }

    pub(crate) fn extender_set_token(&self, token: String) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.config.extender_auth_token = Some(token);
            let _ = self.store.persist_config(&inner.config);
        }
        self.publish();
    }

    pub(crate) fn extender_clear_token(&self) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.config.extender_auth_token = None;
            let _ = self.store.persist_config(&inner.config);
        }
        self.publish();
    }

    pub(crate) fn update_extender_ports(&self, mac: String, ports: Vec<crate::domain::ports::PhysicalPort>) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.extender_ports.insert(mac, ports);
        }
        self.publish();
    }

    pub(crate) fn update_extender_telemetry(&self, mac: String, wireless_clients: Vec<crate::domain::extender::ExtenderClient>) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.extender_clients.insert(mac, wireless_clients);
        }
        self.publish();
    }

    pub(crate) fn update_scan_results(&self, mac: String, networks: Vec<crate::domain::extender::ScannedNetwork>) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.latest_scans.insert(mac, networks);
        }
        self.publish();
    }
}

mod config_init;
