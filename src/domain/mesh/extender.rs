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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtenderClient {
    pub mac: String,
    pub signal_dbm: i32,
    pub distance_m: Option<f32>,
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
    PairRequest { mac: String, model: String, pairing_key: String },
    Auth { token: String },
    Telemetry { mac: String, ports: Vec<crate::domain::ports::PhysicalPort>, wireless_clients: Vec<ExtenderClient> },
    ScanResults { mac: String, networks: Vec<ScannedNetwork> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MeshServerMessage {
    PairStatus { status: String, token: Option<String> },
    AuthResult { success: bool },
    MasterWifi { config: crate::domain::wifi::WifiNetworkConfig },
    CommandScanAirwaves,
}
