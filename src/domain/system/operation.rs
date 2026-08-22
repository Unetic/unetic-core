use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationIntent {
    Wifi {
        ssid: String,
        encryption: String,
        key: Option<String>,
        roaming: crate::domain::RoamingConfig,
    },
    Wan(crate::domain::WanDesired),
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    #[default]
    Wifi,
    Wan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationAccepted {
    pub operation_id: String,
    pub status: OperationStatus,
    pub noop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicOperation {
    pub id: String,
    pub request_id: Option<String>,
    pub source: OperationSource,
    pub kind: String,
    pub status: OperationStatus,
    pub requested_ssid: String,
    #[serde(skip)]
    pub intent: Option<OperationIntent>,
    pub error: Option<crate::domain::errors::LegacyAppError>,
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
    #[serde(skip)]
    pub intent: Option<OperationIntent>,
    pub error: Option<crate::domain::errors::LegacyAppError>,
    pub finished_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionJournal {
    pub schema_version: u32,
    pub operation_id: String,
    pub request_id: String,
    pub source: OperationSource,
    pub base_revision: u64,
    pub target_revision: u64,
    #[serde(default)]
    pub kind: TransactionKind,
    pub old_ssid: String,
    pub new_ssid: String,
    #[serde(default = "crate::domain::wifi::default_wifi_encryption")]
    pub old_encryption: String,
    #[serde(default = "crate::domain::wifi::default_wifi_encryption")]
    pub new_encryption: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_key: Option<String>,
    #[serde(default)]
    pub old_roaming: crate::domain::RoamingConfig,
    #[serde(default)]
    pub new_roaming: crate::domain::RoamingConfig,
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_wan: Option<crate::domain::WanDesired>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_wan: Option<crate::domain::WanDesired>,
    pub phase: OperationStatus,
}
