use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemInfo {
    pub hostname: String,
    pub model: String,
    pub board_name: String,
    pub firmware_version: String,
    pub firmware_revision: String,
    pub target: String,
    pub arch: String,
    pub kernel_version: String,
    pub uptime_secs: u64,
    pub load_average: [f32; 3],
    pub memory_total_kb: u64,
    pub memory_available_kb: u64,
}

impl Default for SystemInfo {
    fn default() -> Self {
        Self {
            hostname: String::new(),
            model: "Generic".into(),
            board_name: String::new(),
            firmware_version: String::new(),
            firmware_revision: String::new(),
            target: String::new(),
            arch: String::new(),
            kernel_version: String::new(),
            uptime_secs: 0,
            load_average: [0.0; 3],
            memory_total_kb: 0,
            memory_available_kb: 0,
        }
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaintenanceState {
    pub enabled: bool,
    pub exiting: bool,
    pub reason: Option<String>,
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

use super::operation::{LastOperation, PublicOperation};
use super::wan::WanPublicState;
use super::wifi::WifiPublicState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicState {
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
    pub last_system_error: Option<crate::domain::errors::LegacyAppError>,
    pub drift: DriftState,
    pub health: HealthState,
    pub registered_devices: Vec<crate::domain::device::RegisteredDevice>,
    pub dns: crate::domain::DnsConfig,
    #[serde(default)]
    pub traffic: crate::domain::traffic::TrafficState,
    #[serde(default)]
    pub ddns_config: crate::domain::DdnsConfig,
    #[serde(default)]
    pub ddns_status: crate::domain::DdnsStatus,
    #[serde(default)]
    pub extenders: Vec<crate::domain::extender::KnownExtender>,
    #[serde(default)]
    pub extender_ports: std::collections::HashMap<String, Vec<crate::domain::ports::PhysicalPort>>,
}
