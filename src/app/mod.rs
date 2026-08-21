use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::Sender,
    },
    time::Duration,
};

use serde_json::json;
use tracing::{error, warn};

use crate::{
    backend::RouterBackend,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{
        DesiredConfig, HealthState, LastOperation, Lifecycle, PublicOperation, PublicState,
        WifiPublicState,
    },
    storage::StateStore,
    switch::SwitchInfo,
};

mod handlers;
mod maintenance;
mod operations;
mod reconcile;
mod recovery;
mod state;
mod wan;

use self::state::{generate_id, snapshot};

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
    pub observed_configs: BTreeMap<String, crate::model::WifiNetworkConfig>,
    pub runtime_healthy: bool,
    pub wan: crate::model::WanPublicState,
    pub active_operation: Option<PublicOperation>,
    pub last_user_operation: Option<LastOperation>,
    pub last_system_error: Option<DomainError>,
    pub event_seq: u64,
    pub boot_id: String,
    pub health: HealthState,
    pub repair_failures: u8,
}

pub struct App {
    pub(crate) backend: Arc<dyn RouterBackend>,
    pub(crate) store: StateStore,
    pub(crate) inner: Mutex<Inner>,
    pub(crate) event_tx: Sender<PublicState>,
    pub(crate) shutdown: AtomicBool,
    pub(crate) op_counter: AtomicUsize,
    pub(crate) timing: Timing,
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
        let (config, lifecycle, init_error) = load_initial_config(backend.as_ref(), &store);
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
            }),
            event_tx,
            shutdown: AtomicBool::new(false),
            op_counter: AtomicUsize::new(1),
            timing,
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

    pub fn switch_get(&self) -> SwitchInfo {
        self.backend
            .read_switch_info()
            .unwrap_or_else(|_| SwitchInfo::generic_software())
    }

    pub fn system_info(&self) -> crate::system::SystemInfo {
        self.backend.read_system_info().unwrap_or_default()
    }

    pub fn devices_list(&self) -> Result<Vec<crate::device::Device>, DomainError> {
        self.backend.read_devices()
    }
}

fn load_initial_config(
    backend: &dyn RouterBackend,
    store: &StateStore,
) -> (DesiredConfig, Lifecycle, Option<DomainError>) {
    match store.load_config() {
        Ok(Some(config)) if config.schema_version == 1 => (config, Lifecycle::Booting, None),
        Ok(Some(_)) => {
            warn!("unsupported desired-state schema");
            let error = DomainError::new(
                ErrorCode::StateCorrupt,
                ErrorStage::Bootstrap,
                "unsupported desired-state schema",
            );
            (DesiredConfig::empty(), Lifecycle::Degraded, Some(error))
        }
        Ok(None) => discover_default_config(backend, store),
        Err(error) => {
            error!(%error, "failed to read desired state from disk");
            (DesiredConfig::empty(), Lifecycle::Degraded, Some(error))
        }
    }
}

fn discover_default_config(
    backend: &dyn RouterBackend,
    store: &StateStore,
) -> (DesiredConfig, Lifecycle, Option<DomainError>) {
    match backend.discover_primary_wifi() {
        Ok(discovered) => {
            let wan = backend
                .discover_primary_wan()
                .map_or_else(|_| crate::model::WanDesired::default(), |w| w.to_desired());
            let config = DesiredConfig::new(discovered.to_network_config(), wan);
            if let Err(error) = store.persist_config(&config) {
                warn!(%error, "failed to persist discovered default config");
                (config, Lifecycle::Degraded, Some(error))
            } else {
                (config, Lifecycle::Booting, None)
            }
        }
        Err(error) => {
            warn!(%error, "failed to discover Wi-Fi interfaces during bootstrap");
            (DesiredConfig::empty(), Lifecycle::NeedsSetup, Some(error))
        }
    }
}
