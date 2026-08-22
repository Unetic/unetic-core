use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum DeviceConnection {
    Wired {
        port_id: u8,
    },
    Wireless {
        signal_dbm: i32,
    },
    ViaExtender {
        extender_mac: String,
        signal_dbm: Option<i32>,
    },
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortForward {
    pub id: String,
    pub external_port: u32,
    pub internal_port: u32,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisteredDevice {
    pub uuid: String,
    pub mac: String,
    pub name: String,
    pub is_static_ip: bool,
    pub port_forwards: Vec<PortForward>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Device {
    pub mac: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    /// Global or ULA IPv6 address (excludes link-local fe80::).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip6: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    pub connection: DeviceConnection,
}

impl Device {
    #[must_use]
    pub fn new(
        mac: impl Into<String>,
        ip: Option<String>,
        ip6: Option<String>,
        hostname: Option<String>,
        connection: DeviceConnection,
    ) -> Self {
        Self {
            mac: mac.into(),
            ip,
            ip6,
            hostname,
            connection,
        }
    }
}
