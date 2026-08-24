use std::{sync::Arc, time::Duration};
use tokio::sync::broadcast::{Receiver, error::RecvError};
use tracing::{error, info, warn};
use unetic_openwrt_sys::{Bridge, Server};

use crate::application::app::App;
use crate::domain::PublicState;
use crate::presentation::api;

pub async fn run_event_loop(
    app: Arc<App>,
    mut event_rx: Receiver<PublicState>,
    is_memory: bool,
) -> anyhow::Result<()> {
    if is_memory {
        info!("memory backend is active; ubus server is disabled");
        wait_for_signal().await?;
        app.shutdown();
        return Ok(());
    }

    let bridge = Bridge::load().map_err(|e| anyhow::anyhow!("failed to initialize ubus: {e}"))?;
    let callback_app = Arc::clone(&app);
    let mut server = bridge
        .server(api::UBUS_METHODS, move |method, request| {
            api::dispatch(&callback_app, method, request)
        })
        .map_err(|e| anyhow::anyhow!("failed to register ubus object 'unetic': {}", e))?;

    info!("Unetic Core is ready on ubus object 'unetic'");

    let mut shutdown_signal = std::pin::pin!(wait_for_signal());
    let mut last_state: Option<serde_json::Value> = None;

    loop {
        tokio::select! {
            _ = &mut shutdown_signal => {
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if let Err(error) = server.poll(0) {
                    error!(%error, "ubus server poll failed");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
            event = event_rx.recv() => {
                let state = match event {
                    Ok(state) => state,
                    Err(RecvError::Lagged(skipped)) => {
                        warn!(skipped, "state event receiver lagged");
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                };

                notify_state(&mut server, &app, &mut last_state, &state);
            }
        }
    }

    info!("shutting down");
    app.shutdown();
    Ok(())
}

fn notify_state(
    server: &mut Server,
    app: &App,
    previous: &mut Option<serde_json::Value>,
    state: &PublicState,
) {
    if !app.has_active_subscribers() {
        *previous = None;
        return;
    }
    let Ok(current) = serde_json::to_value(state) else {
        return;
    };
    match previous.as_ref() {
        Some(previous) => notify_patch(server, previous, &current),
        None => notify_changed(server, state),
    }
    *previous = Some(current);
}

fn notify_changed(server: &mut Server, state: &PublicState) {
    let Ok(json) = serde_json::to_string(state) else {
        error!("failed to serialize state notification");
        return;
    };
    if let Err(error) = server.notify("state.changed", &json) {
        warn!(%error, "failed to publish state.changed");
    }
}

fn notify_patch(server: &mut Server, previous: &serde_json::Value, current: &serde_json::Value) {
    let Some(diff) = crate::application::diff::json_diff(previous, current) else {
        return;
    };
    let Ok(json) = serde_json::to_string(&diff) else {
        return;
    };
    if let Err(error) = server.notify("state.patched", &json) {
        warn!(%error, "failed to publish state.patched");
    }
}

async fn wait_for_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = term.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
    }

    Ok(())
}
