use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRecord {
    pub id: String,
    pub hostname: String,
    pub ip: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsConfig {
    #[serde(default)]
    pub upstream: Vec<String>,
    #[serde(default)]
    pub local_domain: Option<String>,
    #[serde(default = "default_dhcp_start")]
    pub dhcp_start: u32,
    #[serde(default = "default_dhcp_limit")]
    pub dhcp_limit: u32,
    #[serde(default = "default_dhcp_lease_hours")]
    pub dhcp_lease_hours: u32,
    #[serde(default)]
    pub custom_records: Vec<DnsRecord>,
}

fn default_dhcp_start() -> u32 { 100 }
fn default_dhcp_limit() -> u32 { 150 }
fn default_dhcp_lease_hours() -> u32 { 12 }

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            upstream: Vec::new(),
            local_domain: None,
            dhcp_start: 100,
            dhcp_limit: 150,
            dhcp_lease_hours: 12,
            custom_records: Vec::new(),
        }
    }
}
