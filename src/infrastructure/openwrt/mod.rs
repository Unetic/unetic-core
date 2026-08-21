pub(crate) mod devices;
mod native;
mod rpc;
mod switch;
pub(crate) mod system;
pub mod wan;
mod wireless;

pub use self::native::OpenWrtBackend;
pub use self::switch::read_switch_info;
pub use self::wan::{build_wan_staging_values, parse_discovered_wan, parse_wan_runtime_status};
