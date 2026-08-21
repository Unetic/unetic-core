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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WanProtocol {
    #[default]
    Dhcp,
    Static,
    Pppoe,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WanStatus {
    #[default]
    NotConfigured,
    Connecting,
    Connected,
    Disconnected,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WanStaticConfig {
    pub ip_address: String,
    pub netmask: String,
    pub gateway: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WanPppoeConfig {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WanDesired {
    pub present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub proto: WanProtocol,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_mac: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_mtu: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_dns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_config: Option<WanStaticConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pppoe_config: Option<WanPppoeConfig>,
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
    #[serde(default)]
    pub wan: WanDesired,
}

impl DesiredConfig {
    #[must_use]
    pub fn new(ssid: String, targets: Vec<String>, wan: WanDesired) -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            revision: 1,
            wifi: WifiDesired {
                primary: WifiNetworkConfig { ssid, targets },
            },
            wan,
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
            wan: WanDesired::default(),
        }
    }
}

pub mod state;
pub use state::*;

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
pub struct SetWanRequest {
    pub expected_revision: u64,
    pub request_id: String,
    pub wan: WanDesired,
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscoveredWan {
    pub present: bool,
    pub device: Option<String>,
    pub proto: WanProtocol,
    pub custom_mac: Option<String>,
    pub custom_mtu: Option<u16>,
    pub custom_dns: Vec<String>,
    pub static_config: Option<WanStaticConfig>,
    pub pppoe_config: Option<WanPppoeConfig>,
}

impl DiscoveredWan {
    #[must_use]
    pub fn to_desired(&self) -> WanDesired {
        WanDesired {
            present: self.present,
            device: self.device.clone(),
            proto: self.proto,
            custom_mac: self.custom_mac.clone(),
            custom_mtu: self.custom_mtu,
            custom_dns: self.custom_dns.clone(),
            static_config: self.static_config.clone(),
            pppoe_config: self.pppoe_config.clone(),
        }
    }
}
