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
}

impl DesiredConfig {
    #[must_use]
    pub fn new(primary: WifiNetworkConfig, wan: WanDesired) -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            revision: 1,
            wifi: WifiDesired { primary },
            wan,
        }
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            revision: 0,
            wifi: WifiDesired::default(),
            wan: WanDesired::default(),
        }
    }
}
