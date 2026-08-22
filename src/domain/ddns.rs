use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DdnsProvider { #[default] None, Cloudflare, DuckDns }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudflareConfig {
    pub zone_id: String,
    pub record_id: String,
    pub api_token: String,
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DuckDnsConfig {
    pub token: String,
    pub domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DdnsConfig {
    pub enabled: bool,
    pub provider: DdnsProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloudflare: Option<CloudflareConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duckdns: Option<DuckDnsConfig>,
}

/// Runtime-only status, not persisted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DdnsStatus {
    pub last_ip: Option<String>,
    pub last_update_ts: Option<u64>,
    pub last_error: Option<String>,
}
