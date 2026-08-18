#![allow(clippy::pedantic)]

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use anyhow::{Context, Result};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use unetic_core::{App, MemoryBackend, RouterBackend, StateStore, api, openwrt::OpenWrtBackend};
use unetic_openwrt_sys::Bridge;

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

    let backend: Arc<dyn RouterBackend> =
        if std::env::var("UNETIC_BACKEND").as_deref() == Ok("memory") {
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

    if std::env::var("UNETIC_BACKEND").as_deref() == Ok("memory") {
        info!("memory backend is active; ubus server is disabled");
        wait_for_signal().await;
        app.shutdown();
        return Ok(());
    }

    let bridge = Bridge::load().context("failed to load OpenWrt ubus bridge")?;
    let callback_app = Arc::clone(&app);
    let mut server = bridge
        .server(move |method, request| api::dispatch(&callback_app, method, request))
        .context("failed to register ubus object 'unetic'")?;

    let stopping = Arc::new(AtomicBool::new(false));
    install_signal_handlers(Arc::clone(&stopping));

    info!("Unetic Core is ready on ubus object 'unetic'");

    while !stopping.load(Ordering::Relaxed) {
        if let Err(error) = server.poll(100) {
            error!(%error, "ubus server poll failed");
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        while let Ok(state) = event_rx.try_recv() {
            match serde_json::to_string(&state) {
                Ok(json) => {
                    if let Err(error) = server.notify("state.changed", &json) {
                        warn!(%error, "failed to publish state.changed");
                    }
                }
                Err(error) => error!(%error, "failed to serialize state notification"),
            }
        }

        tokio::task::yield_now().await;
    }

    info!("shutting down");
    app.shutdown();
    Ok(())
}

fn install_signal_handlers(stopping: Arc<AtomicBool>) {
    let ctrl_c = Arc::clone(&stopping);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            ctrl_c.store(true, Ordering::Relaxed);
        }
    });

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let term = stopping;
        tokio::spawn(async move {
            if let Ok(mut stream) = signal(SignalKind::terminate()) {
                stream.recv().await;
                term.store(true, Ordering::Relaxed);
            }
        });
    }
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = term.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
