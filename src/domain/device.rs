use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Device {
    pub mac: String,
    pub ip: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    pub connection_type: String,
}

impl Device {
    #[must_use]
    pub fn new(
        mac: impl Into<String>,
        ip: impl Into<String>,
        hostname: Option<String>,
        connection_type: impl Into<String>,
    ) -> Self {
        Self {
            mac: mac.into(),
            ip: ip.into(),
            hostname,
            connection_type: connection_type.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_serialization() {
        let device = Device::new(
            "00:11:22:33:44:55",
            "192.168.1.100",
            Some("my-laptop".into()),
            "Wireless",
        );

        let json = serde_json::to_string(&device).expect("serialize device");
        let deserialized: Device = serde_json::from_str(&json).expect("deserialize device");
        assert_eq!(device, deserialized);
    }

    #[test]
    fn test_device_none_hostname() {
        let device = Device::new("aa:bb:cc:dd:ee:ff", "192.168.1.101", None, "Wired");

        let json = serde_json::to_string(&device).expect("serialize device");
        assert!(!json.contains("hostname"));
        let deserialized: Device = serde_json::from_str(&json).expect("deserialize device");
        assert_eq!(device, deserialized);
        assert_eq!(deserialized.hostname, None);
    }
}
