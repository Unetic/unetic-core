use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::application::app::App;
use crate::domain::extender::MeshMessage;
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
                let mut line = String::new();
                
                let app_clone = Arc::clone(&app);
                
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        let ports = match app_clone.ports_list() {
                            Ok(p) => p,
                            Err(_) => vec![],
                        };
                        let mac = app_clone.system_info().hostname; // use hostname as ID for simplicity
                        let msg = MeshMessage::ExtenderTelemetry { mac, ports };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            if w.write_all(format!("{}\n", json).as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    }
                });

                while let Ok(n) = reader.read_line(&mut line).await {
                    if n == 0 { break; }
                    if let Ok(MeshMessage::MasterWifi { config }) = serde_json::from_str(&line) {
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
                        
                        let wifi_config = {
                            let inner = app_clone.inner.lock().unwrap();
                            inner.config.wifi.primary.clone()
                        };
                        let msg = MeshMessage::MasterWifi { config: wifi_config };
                        if let Ok(json) = serde_json::to_string(&msg) {
                            let _ = w.write_all(format!("{}\n", json).as_bytes()).await;
                        }

                        let mut line = String::new();
                        loop {
                            tokio::select! {
                                Ok(config) = wifi_rx.recv() => {
                                    let msg = MeshMessage::MasterWifi { config };
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
                                            if let Ok(MeshMessage::ExtenderTelemetry { mac, ports }) = serde_json::from_str(&line) {
                                                app_clone.update_extender_ports(mac, ports);
                                            }
                                            line.clear();
                                        }
                                        Err(_) => break,
                                    }
                                }
                            }
                        }
                    });
                }
            }
        });
    }
}
