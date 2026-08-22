#![allow(clippy::pedantic)]

use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
};

use anyhow::{Context, Result};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use unetic_core::{
    App, MemoryBackend, RouterBackend, StateStore, presentation::server::run_event_loop, infrastructure::openwrt::OpenWrtBackend,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("unetic_core=info")),
        )
        .compact()
        .init();

    let state_dir = std::env::var_os("UNETIC_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/unetic"));

    let is_memory = std::env::var("UNETIC_BACKEND").as_deref() == Ok("memory");

    let backend: Arc<dyn RouterBackend> =
        if is_memory {
            warn!("starting with in-memory development backend");
            Arc::new(MemoryBackend::new(
                "Unetic",
                &["default_radio0", "default_radio1"],
            ))
        } else {
            Arc::new(OpenWrtBackend::new().context("failed to initialize OpenWrt backend")?)
        };

    let (event_tx, event_rx) = mpsc::channel();
    let app = App::bootstrap(backend, StateStore::new(state_dir), event_tx);
    app.start_background();

    run_event_loop(app, event_rx, is_memory).await
}
