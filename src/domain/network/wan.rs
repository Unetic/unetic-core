use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WanProtocol {
    #[default]
    Dhcp,
    Static,
    Pppoe,
    Extender,
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
pub struct WanQos {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_kbps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_kbps: Option<u32>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qos: Option<WanQos>,
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
    pub qos: Option<WanQos>,
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
            qos: self.qos.clone(),
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qos: Option<WanQos>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetWanRequest {
    pub expected_revision: u64,
    pub request_id: String,
    pub wan: WanDesired,
}
