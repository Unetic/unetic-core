use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

use crate::model::{
    Lifecycle, OperationSource, OperationStatus, WanProtocol, WanStatus, WifiStatus,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaintenanceState {
    pub enabled: bool,
    pub exiting: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WanPublicState {
    pub present: bool,
    pub proto: WanProtocol,
    pub status: WanStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub netmask: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_address: Option<String>,
    pub uptime_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
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
    pub wan: String,
}

impl Default for HealthState {
    fn default() -> Self {
        Self {
            core: "ok".into(),
            ubus: "unknown".into(),
            rpcd: "unknown".into(),
            wireless: "unknown".into(),
            wan: "unknown".into(),
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
    pub wan: WanPublicState,
    pub active_operation: Option<PublicOperation>,
    pub last_user_operation: Option<LastOperation>,
    pub last_system_error: Option<crate::errors::DomainError>,
    pub drift: DriftState,
    pub health: HealthState,
}
