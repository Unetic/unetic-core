#![allow(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::too_many_lines
)]

pub mod api;
pub mod app;
pub mod backend;
pub mod errors;
pub mod model;
pub mod openwrt;
pub mod storage;
pub mod transaction;

pub use app::{App, Timing};
pub use backend::{MemoryBackend, RouterBackend};
pub use storage::StateStore;
