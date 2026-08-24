use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use tracing::warn;
use tracing_subscriber::EnvFilter;
use unetic_core::{
    App, MemoryBackend, RouterBackend, StateStore, infrastructure::openwrt::OpenWrtBackend,
    presentation::server::run_event_loop,
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

    let state_dir = std::env::var_os("UNETIC_STATE_DIR").map_or_else(
        || PathBuf::from(unetic_core::domain::DEFAULT_STATE_DIR),
        PathBuf::from,
    );

    let is_memory = std::env::var("UNETIC_BACKEND").as_deref() == Ok("memory");

    let backend: Arc<dyn RouterBackend> = if is_memory {
        warn!("starting with in-memory development backend");
        Arc::new(MemoryBackend::new(
            "Unetic",
            &["default_radio0", "default_radio1"],
        ))
    } else {
        Arc::new(OpenWrtBackend::new().context("failed to initialize OpenWrt backend")?)
    };

    let (event_tx, event_rx) = tokio::sync::broadcast::channel(128);
    let app = App::bootstrap(backend, StateStore::new(state_dir), event_tx.clone());
    app.start_background();
    app.start_state_publisher();

    unetic_core::application::ddns_watcher::start_ddns_watcher(
        Arc::clone(&app),
        event_tx.subscribe(),
    );

    if !is_memory {
        unetic_core::infrastructure::openwrt::netlink::start_neighbor_listener(Arc::clone(&app));
    }

    unetic_core::application::mesh_sync::start_mesh_sync(Arc::clone(&app), event_tx.subscribe());

    run_event_loop(app, event_rx, is_memory).await
}
