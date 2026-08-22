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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MeshClientMessage {
    PairRequest { mac: String, model: String, pairing_key: String },
    Auth { token: String },
    Telemetry { mac: String, ports: Vec<crate::domain::ports::PhysicalPort> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MeshServerMessage {
    PairStatus { status: String, token: Option<String> },
    AuthResult { success: bool },
    MasterWifi { config: crate::domain::wifi::WifiNetworkConfig },
}
