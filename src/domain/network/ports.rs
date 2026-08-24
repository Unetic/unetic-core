use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct HardwareOffload {
    pub available: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SwitchState {
    pub hw_offload: HardwareOffload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PortType {
    #[serde(rename = "wan")]
    Wan,
    #[serde(rename = "lan")]
    Lan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[repr(u32)]
pub enum PortSpeed {
    NoLink = 0,
    Speed10 = 10,
    Speed100 = 100,
    Speed1000 = 1000,
    Speed2500 = 2500,
    Speed5000 = 5000,
    Speed10000 = 10000,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortConnection {
    pub device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhysicalPort {
    pub id: String,
    pub port_type: PortType,
    pub speed: PortSpeed,
    pub connections: Vec<PortConnection>,
}
