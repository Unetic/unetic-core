use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TemperatureSource {
    Soc,
    Wifi24,
    Wifi5,
    Switch,
    Ssd,
    Sfp,
    Modem,
    Poe,
    Pcb,
}

impl TemperatureSource {
    #[must_use]
    pub const fn sort_order(self) -> u8 {
        match self {
            Self::Soc => 0,
            Self::Wifi24 => 1,
            Self::Wifi5 => 2,
            Self::Switch => 3,
            Self::Ssd => 4,
            Self::Sfp => 5,
            Self::Modem => 6,
            Self::Poe => 7,
            Self::Pcb => 8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemperatureReading {
    pub source: TemperatureSource,
    pub temp_celsius: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SystemRuntime {
    pub uptime_secs: u64,
    pub load_average: [f32; 3],
    pub memory_total_kb: u64,
    pub memory_available_kb: u64,
    pub storage_total_kb: u64,
    pub storage_available_kb: u64,
    pub temperatures: Vec<TemperatureReading>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SystemState {
    pub info: super::SystemInfo,
    pub runtime: SystemRuntime,
}
