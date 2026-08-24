use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingExtender {
    pub mac: String,
    pub model: String,
    pub pairing_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnownExtender {
    pub mac: String,
    pub ip: String,
    pub model: String,
    pub auth_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicExtender {
    pub mac: String,
    pub ip: String,
    pub model: String,
}

impl From<&KnownExtender> for PublicExtender {
    fn from(extender: &KnownExtender) -> Self {
        Self {
            mac: extender.mac.clone(),
            ip: extender.ip.clone(),
            model: extender.model.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtenderClient {
    pub mac: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_dbm: Option<i32>,
    #[serde(default)]
    pub interface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScannedNetwork {
    pub ssid: String,
    pub bssid: String,
    pub channel: u32,
    pub signal: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MeshClientMessage {
    PairRequest {
        mac: String,
        model: String,
        pairing_key: String,
    },
    Auth {
        token: String,
    },
    Telemetry {
        mac: String,
        ports: Vec<crate::domain::ports::PhysicalPort>,
        wireless_clients: Vec<ExtenderClient>,
    },
    ScanResults {
        mac: String,
        networks: Vec<ScannedNetwork>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MeshServerMessage {
    PairStatus {
        status: String,
        token: Option<String>,
    },
    AuthResult {
        success: bool,
    },
    MasterWifi {
        config: crate::domain::wifi::WifiNetworkConfig,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        roaming: Option<crate::domain::roaming::RoamingConfig>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_master_wifi_message_keeps_roaming_optional() {
        let message: MeshServerMessage = serde_json::from_str(
            r#"{"type":"MasterWifi","config":{"ssid":"Home","encryption":"none","targets":[]}}"#,
        )
        .expect("old mesh message");

        assert!(matches!(
            message,
            MeshServerMessage::MasterWifi { roaming: None, .. }
        ));
    }

    #[test]
    fn master_wifi_serializes_roaming_profile() {
        let message = MeshServerMessage::MasterWifi {
            config: crate::domain::WifiNetworkConfig {
                ssid: "Home".into(),
                encryption: "none".into(),
                key: None,
                targets: Vec::new(),
            },
            roaming: Some(crate::domain::RoamingConfig {
                mode: crate::domain::RoamingMode::Aggressive,
                sensitivity: crate::domain::RoamingSensitivity::High,
            }),
        };
        let value = serde_json::to_value(message).expect("mesh message");

        assert_eq!(value["roaming"]["mode"], "aggressive");
        assert_eq!(value["roaming"]["sensitivity"], "high");
    }

    #[test]
    fn old_extender_telemetry_keeps_new_client_location_optional() {
        let message: MeshClientMessage = serde_json::from_str(
            r#"{"type":"Telemetry","mac":"00:11:22:33:44:55","ports":[],"wireless_clients":[{"mac":"aa:bb:cc:dd:ee:ff","signal_dbm":-50}]}"#,
        )
        .expect("old telemetry message");

        let MeshClientMessage::Telemetry {
            wireless_clients, ..
        } = message
        else {
            panic!("expected telemetry message");
        };
        assert_eq!(wireless_clients[0].interface, None);
        assert_eq!(wireless_clients[0].signal_dbm, Some(-50));
        assert_eq!(wireless_clients[0].network, None);
        assert_eq!(wireless_clients[0].port_id, None);
    }
}
