use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::application::app::App;
use crate::domain::extender::{MeshClientMessage, MeshServerMessage, PendingExtender};
use crate::domain::PublicState;
use crate::domain::wan::WanProtocol;
use crate::domain::wifi::WifiNetworkConfig;

pub fn start_mesh_sync(app: Arc<App>, mut event_rx: tokio::sync::broadcast::Receiver<PublicState>) {
    let is_extender = {
        let inner = app.inner.lock().unwrap();
        inner.config.wan.proto == WanProtocol::Extender
    };

    if is_extender {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let stream = match TcpStream::connect("192.168.1.1:9898").await { 
                    Ok(s) => s, 
                    Err(_) => continue 
                };
                let (r, mut w) = tokio::io::split(stream);
                let mut reader = BufReader::new(r);
                
                let token = {
                    let inner = app.inner.lock().unwrap();
                    inner.config.extender_auth_token.clone()
                };

                let mac = app.system_info().hostname; // using hostname as mac
                let model = app.system_info().model;

                if let Some(t) = token {
                    // Authenticate
                    let msg = MeshClientMessage::Auth { token: t };
                    if let Ok(json) = serde_json::to_string(&msg) {
                        let _ = w.write_all(format!("{}\n", json).as_bytes()).await;
                    }

                    let mut line = String::new();
                    if let Ok(n) = reader.read_line(&mut line).await {
                        if n > 0 {
                            if let Ok(MeshServerMessage::AuthResult { success }) = serde_json::from_str(&line) {
                                if success {
                                    let app_clone = Arc::clone(&app);
                                    let mac_clone = mac.clone();
                                    tokio::spawn(async move {
                                        loop {
                                            tokio::time::sleep(Duration::from_secs(10)).await;
                                            let ports = app_clone.ports_list().unwrap_or_default();
                                            let msg = MeshClientMessage::Telemetry { mac: mac_clone.clone(), ports };
                                            if let Ok(json) = serde_json::to_string(&msg) {
                                                if w.write_all(format!("{}\n", json).as_bytes()).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                    });

                                    line.clear();
                                    while let Ok(n) = reader.read_line(&mut line).await {
                                        if n == 0 { break; }
                                        if let Ok(MeshServerMessage::MasterWifi { config }) = serde_json::from_str(&line) {
                                            let local_wifi = {
                                                let inner = app.inner.lock().unwrap();
                                                inner.config.wifi.primary.clone()
                                            };
                                            if config.ssid != local_wifi.ssid || config.key != local_wifi.key || config.encryption != local_wifi.encryption {
                                                // sync wifi...
                                            }
                                        }
                                        line.clear();
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Pair Request
                    let pairing_key = uuid::Uuid::new_v4().to_string();
                    let msg = MeshClientMessage::PairRequest {
                        mac: mac.clone(),
                        model,
                        pairing_key,
                    };
                    if let Ok(json) = serde_json::to_string(&msg) {
                        let _ = w.write_all(format!("{}\n", json).as_bytes()).await;
                    }

                    let mut line = String::new();
                    if let Ok(n) = reader.read_line(&mut line).await {
                        if n > 0 {
                            if let Ok(MeshServerMessage::PairStatus { status, token: Some(t) }) = serde_json::from_str(&line) {
                                if status == "accepted" {
                                    app.extender_set_token(t);
                                }
                            }
                        }
                    }
                }
            }
        });
    } else {
        // MASTER SERVER
        tokio::spawn(async move {
            let listener = match TcpListener::bind("0.0.0.0:9898").await { Ok(l) => l, Err(_) => return };
            
            let (wifi_tx, _) = tokio::sync::broadcast::channel::<WifiNetworkConfig>(16);
            let wifi_tx_clone = wifi_tx.clone();
            
            tokio::spawn(async move {
                let mut last_wifi: Option<WifiNetworkConfig> = None;
                while let Ok(state) = event_rx.recv().await {
                    let current = state.wifi;
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

            loop {
                if let Ok((stream, _)) = listener.accept().await {
                    let app_clone = Arc::clone(&app);
                    let mut wifi_rx = wifi_tx.subscribe();
                    
                    tokio::spawn(async move {
                        let (r, mut w) = tokio::io::split(stream);
                        let mut reader = BufReader::new(r);
                        
                        let mut line = String::new();
                        if let Ok(n) = reader.read_line(&mut line).await {
                            if n == 0 { return; }
                            
                            if let Ok(msg) = serde_json::from_str::<MeshClientMessage>(&line) {
                                match msg {
                                    MeshClientMessage::PairRequest { mac, model, pairing_key } => {
                                        let is_known = {
                                            let inner = app_clone.inner.lock().unwrap();
                                            inner.config.extenders.iter().any(|e| e.mac == mac)
                                        };
                                        if is_known {
                                            let token = {
                                                let inner = app_clone.inner.lock().unwrap();
                                                inner.config.extenders.iter().find(|e| e.mac == mac).map(|e| e.auth_token.clone()).unwrap()
                                            };
                                            let resp = MeshServerMessage::PairStatus { status: "accepted".to_string(), token: Some(token) };
                                            if let Ok(json) = serde_json::to_string(&resp) {
                                                let _ = w.write_all(format!("{}\n", json).as_bytes()).await;
                                            }
                                        } else {
                                            app_clone.mesh_add_pending(PendingExtender { mac, model, pairing_key });
                                            let resp = MeshServerMessage::PairStatus { status: "pending".to_string(), token: None };
                                            if let Ok(json) = serde_json::to_string(&resp) {
                                                let _ = w.write_all(format!("{}\n", json).as_bytes()).await;
                                            }
                                        }
                                    }
                                    MeshClientMessage::Auth { token } => {
                                        let is_valid = {
                                            let inner = app_clone.inner.lock().unwrap();
                                            inner.config.extenders.iter().any(|e| e.auth_token == token)
                                        };
                                        let resp = MeshServerMessage::AuthResult { success: is_valid };
                                        if let Ok(json) = serde_json::to_string(&resp) {
                                            let _ = w.write_all(format!("{}\n", json).as_bytes()).await;
                                        }
                                        
                                        if is_valid {
                                            let wifi_config = {
                                                let inner = app_clone.inner.lock().unwrap();
                                                inner.config.wifi.primary.clone()
                                            };
                                            let msg = MeshServerMessage::MasterWifi { config: wifi_config };
                                            if let Ok(json) = serde_json::to_string(&msg) {
                                                let _ = w.write_all(format!("{}\n", json).as_bytes()).await;
                                            }

                                            line.clear();
                                            loop {
                                                tokio::select! {
                                                    Ok(config) = wifi_rx.recv() => {
                                                        let msg = MeshServerMessage::MasterWifi { config };
                                                        if let Ok(json) = serde_json::to_string(&msg) {
                                                            if w.write_all(format!("{}\n", json).as_bytes()).await.is_err() {
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    result = reader.read_line(&mut line) => {
                                                        match result {
                                                            Ok(0) => break,
                                                            Ok(_) => {
                                                                if let Ok(MeshClientMessage::Telemetry { mac, ports }) = serde_json::from_str(&line) {
                                                                    app_clone.update_extender_ports(mac, ports);
                                                                }
                                                                line.clear();
                                                            }
                                                            Err(_) => break,
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    });
                }
            }
        });
    }
}
