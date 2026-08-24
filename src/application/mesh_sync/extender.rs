use crate::application::app::App;
use crate::domain::extender::{MeshClientMessage, MeshServerMessage};
use std::{collections::HashMap, sync::Arc};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::warn;

type MeshError = Box<dyn std::error::Error + Send + Sync>;

fn get_master_ip() -> String {
    crate::infrastructure::openwrt::network::default_gateway()
        .unwrap_or_else(|| crate::domain::FALLBACK_MASTER_IP.to_owned())
}

pub fn start_extender_loop(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(crate::domain::MESH_EXTENDER_RETRY_SECS)).await;
            let master_ip = get_master_ip();
            let stream = match TcpStream::connect(format!(
                "{}:{}",
                master_ip,
                crate::domain::MESH_SYNC_PORT
            ))
            .await
            {
                Ok(s) => s,
                Err(_) => continue,
            };

            if handle_connection(stream, Arc::clone(&app)).await.is_err() {
                // Connection lost or error, sleep and retry
            }
        }
    });
}

async fn handle_connection(stream: TcpStream, app: Arc<App>) -> Result<(), MeshError> {
    let (r, mut w) = tokio::io::split(stream);
    let mut reader = BufReader::new(r);

    let token = {
        let inner = app.inner.lock().unwrap();
        inner.config.extender_auth_token.clone()
    };
    let system_info = app.system_info();
    let mac = crate::infrastructure::openwrt::system::local_device_id()
        .unwrap_or_else(|| system_info.hostname.clone());
    let model = system_info.model;

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
    token: String,
) -> Result<(), MeshError> {
    let msg = MeshClientMessage::Auth { token };
    w.write_all(format!("{}\n", serde_json::to_string(&msg)?).as_bytes())
        .await?;

    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Err("Connection closed".into());
    }

    let resp: MeshServerMessage = serde_json::from_str(&line)?;
    if let MeshServerMessage::AuthResult { success } = resp {
        if success {
            run_authenticated_extender(w, reader, app, mac).await?;
        } else {
            app.extender_clear_token()?;
        }
    }
    Ok(())
}

async fn pair(
    w: &mut tokio::io::WriteHalf<TcpStream>,
    reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>,
    app: &Arc<App>,
    mac: &str,
    model: &str,
) -> Result<(), MeshError> {
    let msg = MeshClientMessage::PairRequest {
        mac: mac.to_string(),
        model: model.to_string(),
        pairing_key: app.extender_pairing_key(),
    };
    w.write_all(format!("{}\n", serde_json::to_string(&msg)?).as_bytes())
        .await?;

    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Err("Connection closed".into());
    }

    let resp: MeshServerMessage = serde_json::from_str(&line)?;
    if let MeshServerMessage::PairStatus { status, token } = resp {
        if status == "accepted"
            && let Some(token) = token
        {
            app.extender_set_token(token)?;
        }
    }
    Ok(())
}

async fn run_authenticated_extender(
    w: &mut tokio::io::WriteHalf<TcpStream>,
    reader: &mut BufReader<tokio::io::ReadHalf<TcpStream>>,
    app: &Arc<App>,
    mac: &str,
) -> Result<(), MeshError> {
    let app_clone = Arc::clone(app);
    let mac_clone = mac.to_string();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(32);

    let tx_clone = tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(
                crate::domain::MESH_EXTENDER_TELEMETRY_SECS,
            ))
            .await;
            let ports = app_clone.ports_list().unwrap_or_default();
            let mut clients = clients_on_extender_ports(&ports);
            for (mac, client) in crate::infrastructure::openwrt::devices::get_wireless_clients() {
                clients.insert(
                    mac.clone(),
                    crate::domain::extender::ExtenderClient {
                        mac,
                        signal_dbm: Some(client.signal_dbm),
                        interface: Some(client.interface),
                        network: client.network,
                        port_id: None,
                    },
                );
            }
            let msg = MeshClientMessage::Telemetry {
                mac: mac_clone.clone(),
                ports,
                wireless_clients: clients.into_values().collect(),
            };
            if let Ok(json) = serde_json::to_string(&msg) {
                if tx_clone.send(format!("{}\n", json)).await.is_err() {
                    break;
                }
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
                    handle_server_message(msg, app).await;
                }
                line.clear();
            }
        }
    }
    Ok(())
}

fn clients_on_extender_ports(
    ports: &[crate::domain::ports::PhysicalPort],
) -> HashMap<String, crate::domain::extender::ExtenderClient> {
    let mut clients = HashMap::new();
    for port in ports {
        for connection in &port.connections {
            let Some(mac) = crate::domain::device_inventory::DeviceRuntime::mac_from_id(
                &connection.device_id,
            ) else {
                continue;
            };
            clients.insert(
                mac.clone(),
                crate::domain::extender::ExtenderClient {
                    mac,
                    signal_dbm: None,
                    interface: None,
                    network: None,
                    port_id: Some(port.id.clone()),
                },
            );
        }
    }
    clients
}

async fn handle_server_message(msg: MeshServerMessage, app: &Arc<App>) {
    if let MeshServerMessage::MasterWifi { config, roaming } = msg {
        let app = Arc::clone(app);
        match tokio::task::spawn_blocking(move || app.apply_master_wifi_config(config, roaming))
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warn!(%error, "failed to apply master Wi-Fi configuration"),
            Err(error) => warn!(%error, "mesh Wi-Fi apply task failed"),
        }
    }
}
