use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnownExtender {
    pub mac: String,
    pub ip: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MeshMessage {
    MasterWifi { config: crate::domain::wifi::WifiNetworkConfig },
    ExtenderTelemetry { mac: String, ports: Vec<crate::domain::ports::PhysicalPort> },
}
