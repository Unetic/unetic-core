use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const API_VERSION: u32 = 1;
pub const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Booting,
    Ready,
    Maintenance,
    Degraded,
    NeedsSetup,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WifiStatus {
    Synced,
    Drifted,
    Applying,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Accepted,
    Staging,
    Applying,
    Verifying,
    Persisting,
    Confirming,
    RollingBack,
    Succeeded,
    Failed,
    RollbackFailed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationSource {
    User,
    Reconcile,
    Recovery,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WifiNetworkConfig {
    pub ssid: String,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WifiDesired {
    pub primary: WifiNetworkConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesiredConfig {
    pub schema_version: u32,
    pub revision: u64,
    pub wifi: WifiDesired,
}

impl DesiredConfig {
    #[must_use]
    pub fn new(ssid: String, targets: Vec<String>) -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            revision: 1,
            wifi: WifiDesired {
                primary: WifiNetworkConfig { ssid, targets },
            },
        }
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            revision: 0,
            wifi: WifiDesired {
                primary: WifiNetworkConfig {
                    ssid: String::new(),
                    targets: Vec::new(),
                },
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaintenanceState {
    pub enabled: bool,
    pub exiting: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WifiPublicState {
    pub ssid: String,
    pub targets: Vec<String>,
    pub observed: BTreeMap<String, String>,
    pub status: WifiStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriftState {
    pub detected: bool,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthState {
    pub core: String,
    pub ubus: String,
    pub rpcd: String,
    pub wireless: String,
}

impl Default for HealthState {
    fn default() -> Self {
        Self {
            core: "ok".into(),
            ubus: "unknown".into(),
            rpcd: "unknown".into(),
            wireless: "unknown".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicOperation {
    pub id: String,
    pub request_id: Option<String>,
    pub source: OperationSource,
    pub kind: String,
    pub status: OperationStatus,
    pub requested_ssid: String,
    pub error: Option<crate::errors::DomainError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LastOperation {
    pub id: String,
    pub request_id: Option<String>,
    pub source: OperationSource,
    pub kind: String,
    pub status: OperationStatus,
    pub revision: u64,
    pub requested_ssid: String,
    pub error: Option<crate::errors::DomainError>,
    pub finished_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicState {
    pub api_version: u32,
    pub core_version: String,
    pub boot_id: String,
    pub event_seq: u64,
    pub revision: u64,
    pub lifecycle: Lifecycle,
    pub maintenance: MaintenanceState,
    pub wifi: WifiPublicState,
    pub active_operation: Option<PublicOperation>,
    pub last_user_operation: Option<LastOperation>,
    pub last_system_error: Option<crate::errors::DomainError>,
    pub drift: DriftState,
    pub health: HealthState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionJournal {
    pub schema_version: u32,
    pub operation_id: String,
    pub request_id: String,
    pub source: OperationSource,
    pub base_revision: u64,
    pub target_revision: u64,
    pub old_ssid: String,
    pub new_ssid: String,
    pub targets: Vec<String>,
    pub phase: OperationStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetSsidRequest {
    pub ssid: String,
    pub expected_revision: u64,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationAccepted {
    pub operation_id: String,
    pub status: OperationStatus,
    pub noop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredWifi {
    pub ssid: String,
    pub targets: Vec<String>,
}
