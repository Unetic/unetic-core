use crate::application::app::App;
use crate::domain::PublicState;
use crate::domain::extender::{MeshClientMessage, MeshServerMessage, PendingExtender};
use crate::domain::wifi::WifiDesired;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, warn};

type MeshError = Box<dyn std::error::Error + Send + Sync>;

pub fn start_master_loop(
    app: Arc<App>,
    mut event_rx: tokio::sync::broadcast::Receiver<PublicState>,
) {
    let (wifi_tx, _) = tokio::sync::broadcast::channel::<WifiDesired>(10);
    let wifi_tx_clone = wifi_tx.clone();

    let app_for_event = Arc::clone(&app);
    tokio::spawn(async move {
        let mut last_wifi: Option<WifiDesired> = None;
        while let Ok(_state) = event_rx.recv().await {
            let current = {
                let inner = app_for_event.inner.lock().unwrap();
                inner.config.wifi.clone()
            };
            if Some(&current) != last_wifi.as_ref() {
                let _ = wifi_tx_clone.send(current.clone());
                last_wifi = Some(current);
            }
        }
    });

    tokio::spawn(async move {
        let address = format!("0.0.0.0:{}", crate::domain::MESH_SYNC_PORT);
        let listener = match TcpListener::bind(&address).await {
            Ok(listener) => listener,
            Err(error) => {
                error!(%error, %address, "failed to start mesh listener");
                return;
            }
        };
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    warn!(%error, "failed to accept mesh connection");
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
            };
            let app_ref = Arc::clone(&app);
            let wifi_rx = wifi_tx.subscribe();
            tokio::spawn(async move {
                if let Err(error) = handle_client(stream, app_ref, wifi_rx).await {
                    warn!(%error, %peer, "mesh client disconnected with an error");
                }
            });
        }
    });
}

async fn handle_client(
    stream: TcpStream,
    app: Arc<App>,
    wifi_rx: tokio::sync::broadcast::Receiver<WifiDesired>,
) -> Result<(), MeshError> {
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = BufReader::new(r);

    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(());
    }

    let msg: MeshClientMessage = serde_json::from_str(&line)?;
    match msg {
        MeshClientMessage::PairRequest {
            mac,
            model,
            pairing_key,
        } => {
            handle_pair_request(&mut w, &app, mac, model, pairing_key).await?;
        }
        MeshClientMessage::Auth { token } => {
            if let Some(mac) = handle_auth(&mut w, &app, &token).await? {
                run_authenticated_master(&mut w, &mut reader, &app, wifi_rx, &mac).await?;
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_pair_request(
    w: &mut tokio::io::WriteHalf<TcpStream>,
    app: &Arc<App>,
    mac: String,
    model: String,
    pairing_key: String,
) -> Result<(), MeshError> {
    let mac = mac.to_ascii_lowercase();
    if let Some(token) = app.take_approved_pairing_token(&mac, &pairing_key) {
        let resp = MeshServerMessage::PairStatus {
            status: "accepted".to_string(),
            token: Some(token),
        };
        w.write_all(format!("{}\n", serde_json::to_string(&resp)?).as_bytes())
            .await?;
    } else {
        app.mesh_add_pending(PendingExtender {
            mac,
            model,
            pairing_key,
        });
        let resp = MeshServerMessage::PairStatus {
            status: "pending".to_string(),
            token: None,
        };
        w.write_all(format!("{}\n", serde_json::to_string(&resp)?).as_bytes())
            .await?;
    }
    Ok(())
}

async fn handle_auth(
    w: &mut tokio::io::WriteHalf<TcpStream>,
    app: &Arc<App>,
    token: &str,
) -> Result<Option<String>, MeshError> {
    let authenticated_mac = {
        let inner = app.inner.lock().unwrap();
        inner
            .config
            .extenders
            .iter()
            .find(|extender| extender.auth_token == token)
            .map(|extender| extender.mac.clone())
    };
    let resp = MeshServerMessage::AuthResult {
        success: authenticated_mac.is_some(),
    };
    w.write_all(format!("{}\n", serde_json::to_string(&resp)?).as_bytes())
        .await?;
    Ok(authenticated_mac)
}

async fn run_authenticated_master(
    w: &mut tokio::io::WriteHalf<TcpStream>,
    reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>,
    app: &Arc<App>,
    mut wifi_rx: tokio::sync::broadcast::Receiver<WifiDesired>,
    authenticated_mac: &str,
) -> Result<(), MeshError> {
    let wifi = {
        let inner = app.inner.lock().unwrap();
        inner.config.wifi.clone()
    };
    let msg = MeshServerMessage::MasterWifi {
        config: wifi.primary,
        roaming: Some(wifi.roaming),
    };
    w.write_all(format!("{}\n", serde_json::to_string(&msg)?).as_bytes())
        .await?;

    let mut line = String::new();

    loop {
        tokio::select! {
            result = wifi_rx.recv() => {
                if let Ok(wifi) = result {
                    let msg = MeshServerMessage::MasterWifi {
                        config: wifi.primary,
                        roaming: Some(wifi.roaming),
                    };
                    if w.write_all(format!("{}\n", serde_json::to_string(&msg)?).as_bytes()).await.is_err() { break; }
                }
            }
            result = reader.read_line(&mut line) => {
                if result? == 0 { break; }
                if let Ok(msg) = serde_json::from_str::<MeshClientMessage>(&line) {
                    match msg {
                        MeshClientMessage::Telemetry { ports, wireless_clients, .. } => {
                            app.update_extender_ports(authenticated_mac.to_owned(), ports);
                            app.update_extender_telemetry(authenticated_mac.to_owned(), wireless_clients);
                        }
                        MeshClientMessage::ScanResults { networks, .. } => {
                            app.update_scan_results(authenticated_mac.to_owned(), networks);
                        }
                        _ => {}
                    }
                }
                line.clear();
            }
        }
    }
    Ok(())
}
