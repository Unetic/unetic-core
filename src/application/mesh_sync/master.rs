use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use crate::application::app::App;
use crate::domain::extender::{MeshClientMessage, MeshServerMessage, PendingExtender};
use crate::domain::PublicState;
use crate::domain::wifi::WifiNetworkConfig;

pub fn start_master_loop(app: Arc<App>, mut event_rx: tokio::sync::broadcast::Receiver<PublicState>) {
    let (wifi_tx, _) = tokio::sync::broadcast::channel::<WifiNetworkConfig>(10);
    let wifi_tx_clone = wifi_tx.clone();
    
    let app_for_event = Arc::clone(&app);
    tokio::spawn(async move {
        let mut last_wifi: Option<WifiNetworkConfig> = None;
        while let Ok(_state) = event_rx.recv().await {
            let current = {
                let inner = app_for_event.inner.lock().unwrap();
                inner.config.wifi.primary.clone()
            };
            
            let config = WifiNetworkConfig {
                ssid: current.ssid,
                encryption: current.encryption,
                key: current.key,
                targets: current.targets,
            };
            if Some(&config) != last_wifi.as_ref() {
                let _ = wifi_tx_clone.send(config.clone());
                last_wifi = Some(config);
            }
        }
    });

    tokio::spawn(async move {
        let listener = TcpListener::bind("0.0.0.0:9898").await.unwrap();
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let app_ref = Arc::clone(&app);
                let wifi_rx = wifi_tx.subscribe();
                tokio::spawn(async move {
                    let _ = handle_client(stream, app_ref, wifi_rx).await;
                });
            }
        }
    });
}

async fn handle_client(
    stream: TcpStream, 
    app: Arc<App>, 
    wifi_rx: tokio::sync::broadcast::Receiver<WifiNetworkConfig>
) -> Result<(), Box<dyn std::error::Error>> {
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = BufReader::new(r);
    
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 { return Ok(()); }
    
    let msg: MeshClientMessage = serde_json::from_str(&line)?;
    match msg {
        MeshClientMessage::PairRequest { mac, model, pairing_key } => {
            handle_pair_request(&mut w, &app, mac, model, pairing_key).await?;
        }
        MeshClientMessage::Auth { token } => {
            if handle_auth(&mut w, &app, &token).await? {
                run_authenticated_master(&mut w, &mut reader, &app, wifi_rx).await?;
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
    pairing_key: String
) -> Result<(), Box<dyn std::error::Error>> {
    let (is_known, token) = {
        let inner = app.inner.lock().unwrap();
        let known = inner.config.extenders.iter().find(|e| e.mac == mac);
        (known.is_some(), known.map(|e| e.auth_token.clone()))
    };
    
    if is_known {
        let resp = MeshServerMessage::PairStatus { status: "accepted".to_string(), token };
        w.write_all(format!("{}\n", serde_json::to_string(&resp)?).as_bytes()).await?;
    } else {
        app.mesh_add_pending(PendingExtender { mac, model, pairing_key });
        let resp = MeshServerMessage::PairStatus { status: "pending".to_string(), token: None };
        w.write_all(format!("{}\n", serde_json::to_string(&resp)?).as_bytes()).await?;
    }
    Ok(())
}

async fn handle_auth(
    w: &mut tokio::io::WriteHalf<TcpStream>, 
    app: &Arc<App>, 
    token: &str
) -> Result<bool, Box<dyn std::error::Error>> {
    let is_valid = {
        let inner = app.inner.lock().unwrap();
        inner.config.extenders.iter().any(|e| e.auth_token == token)
    };
    let resp = MeshServerMessage::AuthResult { success: is_valid };
    w.write_all(format!("{}\n", serde_json::to_string(&resp)?).as_bytes()).await?;
    Ok(is_valid)
}

async fn run_authenticated_master(
    w: &mut tokio::io::WriteHalf<TcpStream>, 
    reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>, 
    app: &Arc<App>, 
    mut wifi_rx: tokio::sync::broadcast::Receiver<WifiNetworkConfig>
) -> Result<(), Box<dyn std::error::Error>> {
    let wifi_config = {
        let inner = app.inner.lock().unwrap();
        inner.config.wifi.primary.clone()
    };
    let msg = MeshServerMessage::MasterWifi { config: wifi_config };
    w.write_all(format!("{}\n", serde_json::to_string(&msg)?).as_bytes()).await?;

    let mut rrm_rx = app.rrm_tx.subscribe();
    let mut line = String::new();
    
    loop {
        tokio::select! {
            result = wifi_rx.recv() => {
                if let Ok(config) = result {
                    let msg = MeshServerMessage::MasterWifi { config };
                    if w.write_all(format!("{}\n", serde_json::to_string(&msg)?).as_bytes()).await.is_err() { break; }
                }
            }
            result = rrm_rx.recv() => {
                if result.is_ok() {
                    let msg = MeshServerMessage::CommandScanAirwaves;
                    if w.write_all(format!("{}\n", serde_json::to_string(&msg)?).as_bytes()).await.is_err() { break; }
                }
            }
            result = reader.read_line(&mut line) => {
                if result? == 0 { break; }
                if let Ok(msg) = serde_json::from_str::<MeshClientMessage>(&line) {
                    match msg {
                        MeshClientMessage::Telemetry { mac, ports, wireless_clients } => {
                            app.update_extender_ports(mac.clone(), ports);
                            app.update_extender_telemetry(mac, wireless_clients);
                        }
                        MeshClientMessage::ScanResults { mac, networks } => {
                            app.update_scan_results(mac, networks);
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
