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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RadioChannelConfig {
    pub target: String,
    pub channel: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeshBackhaulConfig {
    pub enabled: bool,
    pub backhaul_target: String,
    pub client_target: String,
    #[serde(default = "default_true")]
    pub hidden: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WifiDesired {
    pub primary: WifiNetworkConfig,
    #[serde(default)]
    pub roaming: crate::domain::roaming::RoamingConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backhaul: Option<MeshBackhaulConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub radio_channels: Vec<RadioChannelConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscoveredWifi {
    pub ssid: String,
    pub encryption: String,
    pub key: Option<String>,
    pub targets: Vec<String>,
    pub backhaul: Option<MeshBackhaulConfig>,
    pub radio_channels: Vec<RadioChannelConfig>,
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
    #[serde(default)]
    pub roaming: crate::domain::roaming::RoamingConfig,
    #[serde(default)]
    pub roaming_runtime: crate::domain::roaming::RoamingRuntime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backhaul: Option<MeshBackhaulConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub radio_channels: Vec<RadioChannelConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetWifiConfigRequest {
    pub ssid: String,
    #[serde(default = "default_wifi_encryption")]
    pub encryption: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roaming: Option<crate::domain::roaming::RoamingConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backhaul: Option<MeshBackhaulConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub radio_channels: Vec<RadioChannelConfig>,
    pub expected_revision: u64,
    pub request_id: String,
}

impl SetWifiConfigRequest {
    #[must_use]
    pub fn new(
        ssid: impl Into<String>,
        encryption: impl Into<String>,
        key: Option<String>,
        expected_revision: u64,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            ssid: ssid.into(),
            encryption: encryption.into(),
            key,
            roaming: None,
            backhaul: None,
            radio_channels: Vec::new(),
            expected_revision,
            request_id: request_id.into(),
        }
    }
}

impl Default for SetWifiConfigRequest {
    fn default() -> Self {
        Self {
            ssid: String::new(),
            encryption: "none".into(),
            key: None,
            roaming: None,
            backhaul: None,
            radio_channels: Vec::new(),
            expected_revision: 0,
            request_id: String::new(),
        }
    }
}

pub type SetSsidRequest = SetWifiConfigRequest;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_desired_and_request_json_use_compatible_roaming_defaults() {
        let desired: WifiDesired = serde_json::from_str(
            r#"{"primary":{"ssid":"Home","encryption":"none","targets":["radio0"]}}"#,
        )
        .expect("old desired Wi-Fi state");
        assert_eq!(desired.roaming, Default::default());

        let request: SetWifiConfigRequest = serde_json::from_str(
            r#"{"ssid":"Home","encryption":"none","expected_revision":1,"request_id":"old"}"#,
        )
        .expect("old Wi-Fi request");
        assert_eq!(request.roaming, None);
    }
}
