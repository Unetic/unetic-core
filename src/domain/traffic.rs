use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct IfaceStats { pub rx_bps: u64, pub tx_bps: u64 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DeviceStats { pub rx_bps: u64, pub tx_bps: u64 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrafficState {
    pub ifaces: HashMap<String, IfaceStats>,
    pub devices: HashMap<String, DeviceStats>,
}
