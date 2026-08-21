mod native;
mod rpc;
mod switch;
mod wan;
mod wireless;

pub use self::native::OpenWrtBackend;
pub use self::switch::read_switch_info;
