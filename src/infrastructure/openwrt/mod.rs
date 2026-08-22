pub(crate) mod devices;
mod native;
mod rpc;
pub(crate) mod ports;

pub(crate) mod system;
pub mod netlink;
pub mod wan;
mod wireless;
pub mod dns;
pub mod traffic;
pub use self::native::OpenWrtBackend;

pub use self::wan::{build_wan_staging_values, parse_discovered_wan, parse_wan_runtime_status};
