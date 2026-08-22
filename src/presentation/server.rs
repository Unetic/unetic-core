use std::{sync::Arc, time::Duration};
use tokio::sync::broadcast::Receiver;
use tracing::{error, info, warn};
use unetic_openwrt_sys::Bridge;

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

    let bridge =
        Bridge::load().map_err(|e| anyhow::anyhow!("failed to load OpenWrt ubus bridge: {}", e))?;
    let callback_app = Arc::clone(&app);
    let mut server = bridge
        .server(move |method, request| api::dispatch(&callback_app, method, request))
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

                while let Ok(state) = event_rx.try_recv() {
                    match serde_json::to_string(&state) {
                        Ok(json) => {
                            if let Err(error) = server.notify("state.changed", &json) {
                                warn!(%error, "failed to publish state.changed");
                            }
                        }
                        Err(error) => error!(%error, "failed to serialize state notification"),
                    }

                    if app.has_active_subscribers() {
                        if let Ok(state_val) = serde_json::to_value(&state) {
                            if let Some(last) = &last_state {
                                if let Some(diff) = crate::application::diff::json_diff(last, &state_val) {
                                    if let Ok(diff_json) = serde_json::to_string(&diff) {
                                        if let Err(error) = server.notify("state.patched", &diff_json) {
                                            warn!(%error, "failed to publish state.patched");
                                        }
                                    }
                                }
                            }
                            last_state = Some(state_val);
                        }
                    } else {
                        last_state = None;
                    }
                }
            }
        }
    }

    info!("shutting down");
    app.shutdown();
    Ok(())
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
