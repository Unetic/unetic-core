use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use crate::application::app::App;
use crate::domain::extender::{MeshClientMessage, MeshServerMessage};

fn get_master_ip() -> String {
    let output = std::process::Command::new("ip")
        .args(&["route", "show", "default"])
        .output()
        .unwrap_or_else(|_| std::process::Command::new("true").output().unwrap());
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(gw) = stdout.split_whitespace().nth(2) {
        return gw.to_string();
    }
    crate::domain::FALLBACK_MASTER_IP.to_string()
}

pub fn start_extender_loop(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(crate::domain::MESH_EXTENDER_RETRY_SECS)).await;
            let master_ip = get_master_ip();
            let stream = match TcpStream::connect(format!("{}:{}", master_ip, crate::domain::MESH_SYNC_PORT)).await {
                Ok(s) => s,
                Err(_) => continue,
            };
            
            if let Err(_) = handle_connection(stream, Arc::clone(&app)).await {
                // Connection lost or error, sleep and retry
            }
        }
    });
}

async fn handle_connection(stream: TcpStream, app: Arc<App>) -> Result<(), Box<dyn std::error::Error>> {
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = BufReader::new(r);
    
    let token = {
        let inner = app.inner.lock().unwrap();
        inner.config.extender_auth_token.clone()
    };
    let mac = app.system_info().hostname.clone();
    let model = app.system_info().model.clone();

    if let Some(t) = token {
        authenticate(&mut w, &mut reader, &app, &mac, t).await?;
    } else {
        pair(&mut w, &mut reader, &app, &mac, &model).await?;
    }
    
    Ok(())
}

async fn authenticate(
    w: &mut tokio::io::WriteHalf<TcpStream>, 
    reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>, 
    app: &Arc<App>, 
    mac: &str, 
    token: String
) -> Result<(), Box<dyn std::error::Error>> {
    let msg = MeshClientMessage::Auth { token };
    w.write_all(format!("{}\n", serde_json::to_string(&msg)?).as_bytes()).await?;

    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 { return Err("Connection closed".into()); }
    
    let resp: MeshServerMessage = serde_json::from_str(&line)?;
    if let MeshServerMessage::AuthResult { success } = resp {
        if success {
            run_authenticated_extender(w, reader, app, mac).await?;
        } else {
            let mut inner = app.inner.lock().unwrap();
            inner.config.extender_auth_token = None;
        }
    }
    Ok(())
}

async fn pair(
    w: &mut tokio::io::WriteHalf<TcpStream>, 
    reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>, 
    app: &Arc<App>, 
    mac: &str, 
    model: &str
) -> Result<(), Box<dyn std::error::Error>> {
    let msg = MeshClientMessage::PairRequest { mac: mac.to_string(), model: model.to_string(), pairing_key: "0000".to_string() };
    w.write_all(format!("{}\n", serde_json::to_string(&msg)?).as_bytes()).await?;
    
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 { return Err("Connection closed".into()); }
    
    let resp: MeshServerMessage = serde_json::from_str(&line)?;
    if let MeshServerMessage::PairStatus { status, token } = resp {
        if status == "accepted" && token.is_some() {
            let mut inner = app.inner.lock().unwrap();
            inner.config.extender_auth_token = token;
        }
    }
    Ok(())
}

async fn run_authenticated_extender(
    w: &mut tokio::io::WriteHalf<TcpStream>, 
    reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>, 
    app: &Arc<App>, 
    mac: &str
) -> Result<(), Box<dyn std::error::Error>> {
    let app_clone = Arc::clone(app);
    let mac_clone = mac.to_string();
    
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(32);
    
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(crate::domain::MESH_EXTENDER_TELEMETRY_SECS)).await;
            let ports = app_clone.ports_list().unwrap_or_default();
            let wireless_clients = crate::infrastructure::openwrt::devices::get_wireless_clients()
                .into_iter()
                .map(|(c_mac, (signal_dbm, distance_m))| crate::domain::extender::ExtenderClient {
                    mac: c_mac,
                    signal_dbm,
                    distance_m: Some(distance_m),
                })
                .collect();
            let msg = MeshClientMessage::Telemetry { mac: mac_clone.clone(), ports, wireless_clients };
            if let Ok(json) = serde_json::to_string(&msg) {
                if tx_clone.send(format!("{}\n", json)).await.is_err() { break; }
            }
        }
    });

    let mut line = String::new();
    loop {
        tokio::select! {
            msg = rx.recv() => {
                if let Some(data) = msg {
                    if w.write_all(data.as_bytes()).await.is_err() { break; }
                }
            }
            result = reader.read_line(&mut line) => {
                if result? == 0 { break; }
                if let Ok(msg) = serde_json::from_str::<MeshServerMessage>(&line) {
                    handle_server_message(msg, app, &tx, mac).await;
                }
                line.clear();
            }
        }
    }
    Ok(())
}

async fn handle_server_message(
    msg: MeshServerMessage, 
    app: &Arc<App>, 
    tx: &tokio::sync::mpsc::Sender<String>,
    mac: &str
) {
    match msg {
        MeshServerMessage::MasterWifi { config } => {
            {
                let mut inner = app.inner.lock().unwrap();
                inner.config.wifi.primary = config;
            }
            // Triggers wifi_set locally if needed
            let _ = tokio::task::spawn_blocking(|| {
                let _ = std::process::Command::new("wifi").output();
            }).await;
        }
        MeshServerMessage::CommandScanAirwaves => {
            let tx_clone = tx.clone();
            let mac_clone = mac.to_string();
            tokio::spawn(async move {
                let networks = tokio::task::spawn_blocking(|| {
                    // Placeholder for real scan
                    vec![]
                }).await.unwrap_or_default();
                
                let msg = MeshClientMessage::ScanResults { mac: mac_clone, networks };
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = tx_clone.send(format!("{}\n", json)).await;
                }
            });
        }
        _ => {}
    }
}
