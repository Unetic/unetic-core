use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WifiStatus {
    Synced,
    Drifted,
    Applying,
    Unknown,
}

pub fn default_wifi_encryption() -> String {
    "none".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WifiNetworkConfig {
    pub ssid: String,
    #[serde(default = "default_wifi_encryption")]
    pub encryption: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub targets: Vec<String>,
}

impl Default for WifiNetworkConfig {
    fn default() -> Self {
        Self {
            ssid: String::new(),
            encryption: "none".into(),
            key: None,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WifiDesired {
    pub primary: WifiNetworkConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredWifi {
    pub ssid: String,
    pub encryption: String,
    pub key: Option<String>,
    pub targets: Vec<String>,
}

impl Default for DiscoveredWifi {
    fn default() -> Self {
        Self {
            ssid: String::new(),
            encryption: "none".into(),
            key: None,
            targets: Vec::new(),
        }
    }
}

impl DiscoveredWifi {
    #[must_use]
    pub fn to_network_config(&self) -> WifiNetworkConfig {
        WifiNetworkConfig {
            ssid: self.ssid.clone(),
            encryption: self.encryption.clone(),
            key: self.key.clone(),
            targets: self.targets.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WifiPublicState {
    pub ssid: String,
    #[serde(default = "default_wifi_encryption")]
    pub encryption: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub targets: Vec<String>,
    pub observed: BTreeMap<String, String>,
    pub status: WifiStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetWifiConfigRequest {
    pub ssid: String,
    #[serde(default = "default_wifi_encryption")]
    pub encryption: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub expected_revision: u64,
    pub request_id: String,
}

pub type SetSsidRequest = SetWifiConfigRequest;
