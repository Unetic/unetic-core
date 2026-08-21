use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemInfo {
    pub hostname: String,
    pub model: String,
    pub board_name: String,
    pub firmware_version: String,
    pub firmware_revision: String,
    pub target: String,
    pub arch: String,
    pub kernel_version: String,
    pub uptime_secs: u64,
    pub load_average: [f32; 3],
    pub memory_total_kb: u64,
    pub memory_available_kb: u64,
}

impl Default for SystemInfo {
    fn default() -> Self {
        Self {
            hostname: String::new(),
            model: "Generic".into(),
            board_name: String::new(),
            firmware_version: String::new(),
            firmware_revision: String::new(),
            target: String::new(),
            arch: String::new(),
            kernel_version: String::new(),
            uptime_secs: 0,
            load_average: [0.0; 3],
            memory_total_kb: 0,
            memory_available_kb: 0,
        }
    }
}
