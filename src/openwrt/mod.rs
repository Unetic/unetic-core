pub(crate) mod devices;
mod native;
mod rpc;
mod switch;
pub(crate) mod system;
mod wan;
mod wireless;

pub use self::native::OpenWrtBackend;
pub use self::switch::read_switch_info;
