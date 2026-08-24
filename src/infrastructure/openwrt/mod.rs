mod device_config;
pub(crate) mod devices;
mod native;
pub(crate) mod ports;
mod rpc;

pub mod dns;
pub mod netlink;
pub(crate) mod system;
mod temperature;
pub mod traffic;
pub mod wan;
mod wireless;
pub use self::native::OpenWrtBackend;

pub use self::wan::{build_wan_staging_values, parse_discovered_wan, parse_wan_runtime_status};
pub mod network;
pub mod qos;
