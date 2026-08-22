use serde::{Deserialize, Serialize};

use super::system::STATE_SCHEMA_VERSION;
use super::wan::WanDesired;
use super::wifi::{WifiDesired, WifiNetworkConfig};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesiredConfig {
    pub schema_version: u32,
    pub revision: u64,
    pub wifi: WifiDesired,
    #[serde(default)]
    pub wan: WanDesired,
    #[serde(default)]
    pub registered_devices: Vec<crate::domain::device::RegisteredDevice>,
    #[serde(default)]
    pub dns: crate::domain::DnsConfig,
    #[serde(default)]
    pub ddns: crate::domain::DdnsConfig,
    #[serde(default)]
    pub extenders: Vec<crate::domain::extender::KnownExtender>,
    #[serde(default)]
    pub extender_auth_token: Option<String>,
}

impl DesiredConfig {
    #[must_use]
    pub fn new(primary: WifiNetworkConfig, wan: WanDesired) -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            revision: 1,
            wifi: WifiDesired { primary },
            wan,
            registered_devices: Vec::new(),
            dns: crate::domain::DnsConfig::default(),
            ddns: crate::domain::DdnsConfig::default(),
            extenders: Vec::new(),
            extender_auth_token: None,
        }
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            revision: 0,
            wifi: WifiDesired::default(),
            wan: WanDesired::default(),
            registered_devices: Vec::new(),
            dns: crate::domain::DnsConfig::default(),
            ddns: crate::domain::DdnsConfig::default(),
            extenders: Vec::new(),
            extender_auth_token: None,
        }
    }
}
