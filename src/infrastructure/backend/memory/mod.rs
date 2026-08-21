use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
};

use crate::domain::{WanDesired, WanPublicState, WanStatus, WifiNetworkConfig};

mod mock;
mod router;
mod wan;

#[derive(Debug, Clone, Copy, Default)]
pub struct FailurePlan {
    pub fail_stage: bool,
    pub fail_apply: bool,
    pub fail_confirm: bool,
    pub fail_rollback: bool,
    pub fail_candidate_verify: bool,
    pub runtime_unhealthy: bool,
}

#[derive(Debug)]
pub(crate) struct MemoryState {
    pub(crate) committed: BTreeMap<String, WifiNetworkConfig>,
    pub(crate) wan_committed: WanDesired,
    pub(crate) sessions: HashMap<String, BTreeMap<String, WifiNetworkConfig>>,
    pub(crate) wan_sessions: HashMap<String, WanDesired>,
    pub(crate) rollback_snapshots: HashMap<String, BTreeMap<String, WifiNetworkConfig>>,
    pub(crate) wan_rollback_snapshots: HashMap<String, WanDesired>,
    pub(crate) wan_runtime: WanPublicState,
    pub(crate) next_session: u64,
    pub(crate) failure: FailurePlan,
}

#[derive(Debug)]
pub struct MemoryBackend {
    pub(crate) state: Mutex<MemoryState>,
}

impl MemoryBackend {
    #[must_use]
    pub fn new(ssid: &str, targets: &[&str]) -> Self {
        Self::with_wan(ssid, targets, WanDesired::default())
    }

    #[must_use]
    pub fn with_wan(ssid: &str, targets: &[&str], wan: WanDesired) -> Self {
        let wifi = WifiNetworkConfig {
            ssid: ssid.to_owned(),
            encryption: "none".into(),
            key: None,
            targets: targets.iter().map(|s| (*s).to_owned()).collect(),
        };
        Self::with_wifi_and_wan(wifi, targets, wan)
    }

    #[must_use]
    pub fn with_wifi_and_wan(wifi: WifiNetworkConfig, targets: &[&str], wan: WanDesired) -> Self {
        let committed = targets
            .iter()
            .map(|target| {
                let mut cfg = wifi.clone();
                cfg.targets = vec![(*target).to_owned()];
                ((*target).to_owned(), cfg)
            })
            .collect();
        let wan_runtime = WanPublicState {
            present: wan.present,
            proto: wan.proto,
            status: if wan.present {
                WanStatus::Connected
            } else {
                WanStatus::NotConfigured
            },
            device: wan.device.clone(),
            ip_address: if wan.present {
                Some("203.0.113.10".into())
            } else {
                None
            },
            netmask: if wan.present {
                Some("255.255.255.0".into())
            } else {
                None
            },
            gateway: if wan.present {
                Some("203.0.113.1".into())
            } else {
                None
            },
            dns: if wan.present {
                vec!["1.1.1.1".into(), "1.0.0.1".into()]
            } else {
                Vec::new()
            },
            mac_address: Some("00:11:22:33:44:55".into()),
            uptime_secs: 1200,
            error_reason: None,
        };
        Self {
            state: Mutex::new(MemoryState {
                committed,
                wan_committed: wan,
                sessions: HashMap::new(),
                wan_sessions: HashMap::new(),
                rollback_snapshots: HashMap::new(),
                wan_rollback_snapshots: HashMap::new(),
                wan_runtime,
                next_session: 1,
                failure: FailurePlan::default(),
            }),
        }
    }

    pub fn set_failure_plan(&self, failure: FailurePlan) {
        self.state.lock().expect("memory backend poisoned").failure = failure;
    }

    #[must_use]
    pub fn committed_configs(&self) -> BTreeMap<String, WifiNetworkConfig> {
        self.state
            .lock()
            .expect("memory backend poisoned")
            .committed
            .clone()
    }

    #[must_use]
    pub fn committed_ssids(&self) -> BTreeMap<String, String> {
        self.state
            .lock()
            .expect("memory backend poisoned")
            .committed
            .iter()
            .map(|(k, v)| (k.clone(), v.ssid.clone()))
            .collect()
    }

    pub fn external_set(&self, target: &str, config: WifiNetworkConfig) {
        self.state
            .lock()
            .expect("memory backend poisoned")
            .committed
            .insert(target.to_owned(), config);
    }

    pub fn external_set_ssid(&self, target: &str, ssid: &str) {
        let mut state = self.state.lock().expect("memory backend poisoned");
        let prev = state.committed.get(target).cloned().unwrap_or_default();
        state.committed.insert(
            target.to_owned(),
            WifiNetworkConfig {
                ssid: ssid.to_owned(),
                ..prev
            },
        );
    }
}
