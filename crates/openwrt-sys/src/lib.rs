//! Rust bindings for the OpenWrt ubus server runtime.
//!
//! The production binary links to the system `libubus`, `libubox`, and
//! `libblobmsg_json` libraries supplied by OpenWrt. Host builds keep a stub so
//! the memory backend can be tested without an OpenWrt SDK.

#![allow(unsafe_code)]
#![allow(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions
)]

#[cfg(target_env = "musl")]
mod openwrt;
#[cfg(not(target_env = "musl"))]
mod stub;

#[cfg(target_env = "musl")]
pub use openwrt::{Bridge, BridgeError, Server};
#[cfg(not(target_env = "musl"))]
pub use stub::{Bridge, BridgeError, Server};
