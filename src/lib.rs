#![allow(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::too_many_lines
)]

pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod presentation;

pub use application::{App, Timing};
pub use domain::{
    Device, SwitchArchitecture, SwitchFeatureStatus, SwitchFeatures, SwitchInfo, SwitchSocInfo,
    SystemInfo,
};
pub use infrastructure::{MemoryBackend, RouterBackend, StateStore, openwrt};
pub use presentation::api;
