pub const MESH_SYNC_PORT: u16 = 9898;
pub const FALLBACK_MASTER_IP: &str = "192.168.1.1";
pub const DEFAULT_STATE_DIR: &str = "/etc/unetic";

pub const CLOUDFLARE_API_URL_PREFIX: &str = "https://api.cloudflare.com/client/v4/zones";
pub const CLOUDFLARE_DEFAULT_TTL: u32 = 60;
pub const DUCKDNS_API_URL_PREFIX: &str = "https://www.duckdns.org/update";

pub const MESH_EXTENDER_RETRY_SECS: u64 = 3;
pub const MESH_EXTENDER_TELEMETRY_SECS: u64 = 10;
pub const TRAFFIC_SAMPLING_INTERVAL_SECS: u64 = 1;
pub const MAX_WAN_QOS_KBPS: u32 = 10_000_000;
pub const HTTP_TIMEOUT_SECS: u64 = 15;
pub const PING_COUNT: &str = "4";
pub const PING_TIMEOUT_SECS: &str = "2";
