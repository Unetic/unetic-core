use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    application::app::App,
    domain::{
        device::{PortForward, PortForwardProtocol, RegisteredDevice},
        device_inventory::DeviceRuntime,
    },
};

#[derive(Serialize)]
struct DeviceView {
    #[serde(flatten)]
    runtime: DeviceRuntime,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    is_static_ip: bool,
    port_forwards: Vec<PortForward>,
}

#[derive(Deserialize)]
struct RegisterRequest {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct UpdateRequest {
    id: String,
    name: String,
    is_static_ip: bool,
}

#[derive(Deserialize)]
struct NewPortForward {
    external_port: u32,
    internal_port: u32,
    protocol: PortForwardProtocol,
}

#[derive(Deserialize)]
struct AddPortForwardRequest {
    id: String,
    port_forward: NewPortForward,
}

#[derive(Deserialize)]
struct RemovePortForwardRequest {
    id: String,
    pf_id: String,
}

#[derive(Deserialize)]
struct UnregisterRequest {
    id: String,
}

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "devices.list" => list_devices(app),
        "devices.register" => {
            let request: RegisterRequest = serde_json::from_value(request).map_err(|_| 1_u32)?;
            let mac = device_mac(app, &request.id)?;
            app.register_device(RegisteredDevice {
                id: request.id,
                mac,
                name: request.name,
                is_static_ip: false,
                port_forwards: Vec::new(),
            })
            .map(|()| json!({}))
            .map_err(|_| 1)
        }
        "devices.update" => {
            let request: UpdateRequest = serde_json::from_value(request).map_err(|_| 1_u32)?;
            let mut device = registered_device(app, &request.id)?;
            device.name = request.name;
            device.is_static_ip = request.is_static_ip;
            app.update_device(&device.id, device.clone())
                .map(|()| json!({}))
                .map_err(|_| 1)
        }
        "devices.unregister" => {
            let request: UnregisterRequest = serde_json::from_value(request).map_err(|_| 1_u32)?;
            app.unregister_device(&request.id)
                .map(|()| json!({}))
                .map_err(|_| 1)
        }
        "devices.add_port_forward" => {
            let request: AddPortForwardRequest =
                serde_json::from_value(request).map_err(|_| 1_u32)?;
            let rule = PortForward {
                id: uuid::Uuid::new_v4().to_string(),
                external_port: request.port_forward.external_port,
                internal_port: request.port_forward.internal_port,
                protocol: request.port_forward.protocol,
            };
            let device = registered_device(app, &request.id)?;
            app.add_port_forward(&device.id, rule)
                .map(|()| json!({}))
                .map_err(|_| 1)
        }
        "devices.remove_port_forward" => {
            let request: RemovePortForwardRequest =
                serde_json::from_value(request).map_err(|_| 1_u32)?;
            let device = registered_device(app, &request.id)?;
            app.remove_port_forward(&device.id, &request.pf_id)
                .map(|()| json!({}))
                .map_err(|_| 1)
        }
        _ => Err(1),
    }
}

fn list_devices(app: &App) -> Result<Value, u32> {
    let state = app.state();
    let registered = state.registered_devices;
    let devices = state.devices;
    let views: Vec<DeviceView> = devices
        .into_iter()
        .map(|runtime| {
            let registration = registered
                .iter()
                .find(|registered| registered.mac.eq_ignore_ascii_case(&runtime.device.mac));
            DeviceView {
                runtime,
                name: registration.map(|registered| registered.name.clone()),
                is_static_ip: registration.is_some_and(|registered| registered.is_static_ip),
                port_forwards: registration
                    .map(|registered| registered.port_forwards.clone())
                    .unwrap_or_default(),
            }
        })
        .collect();
    Ok(json!(views))
}

fn registered_device(app: &App, id: &str) -> Result<RegisteredDevice, u32> {
    app.state()
        .registered_devices
        .into_iter()
        .find(|device| device.id == id)
        .ok_or(1)
}

fn device_mac(app: &App, id: &str) -> Result<String, u32> {
    app.state()
        .devices
        .into_iter()
        .find(|device| device.id == id)
        .map(|device| device.device.mac)
        .ok_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        infrastructure::{backend::memory::MemoryBackend, storage::StateStore},
        presentation::api,
    };

    fn app() -> Arc<App> {
        let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let state_dir =
            std::env::temp_dir().join(format!("unetic-device-api-{}", uuid::Uuid::new_v4()));
        App::bootstrap(backend, StateStore::new(state_dir), tx)
    }

    fn call(app: &Arc<App>, method: &str, request: Value) -> Value {
        let mut request = request;
        request["idempotence_token"] = json!(uuid::Uuid::new_v4().to_string());
        serde_json::from_str(&api::dispatch(app, method, &request.to_string()))
            .expect("valid API response")
    }

    #[test]
    fn register_update_and_forward_use_public_dtos() {
        let app = app();
        let response = call(
            &app,
            "devices.register",
            json!({"id": "device-001122334455", "name": "Phone"}),
        );
        assert_eq!(response["error"], 0);

        let list = call(&app, "devices.list", json!({}));
        let device = list["result"]
            .as_array()
            .and_then(|devices| devices.first())
            .expect("registered device in list");
        let id = device["id"].as_str().expect("device ID");
        assert_eq!(device["name"], "Phone");
        assert_eq!(device["registered"], true);
        assert!(device.get("uuid").is_none());

        let response = call(
            &app,
            "devices.update",
            json!({"id": id, "name": "Work phone", "is_static_ip": true}),
        );
        assert_eq!(response["error"], 0);

        let response = call(
            &app,
            "devices.add_port_forward",
            json!({
                "id": id,
                "port_forward": {
                    "external_port": 8443,
                    "internal_port": 443,
                    "protocol": "TCP"
                }
            }),
        );
        assert_eq!(response["error"], 0);

        let state = app.state();
        let registered = state.registered_devices.first().expect("registered device");
        assert_eq!(registered.name, "Work phone");
        assert!(registered.is_static_ip);
        assert_eq!(registered.port_forwards.len(), 1);
    }

    #[test]
    fn rejects_legacy_or_incomplete_device_payloads() {
        let app = app();
        let response = call(
            &app,
            "devices.register",
            json!({"uuid": "client-id", "name": "Missing MAC"}),
        );
        assert_eq!(response["error"], 1);

        let response = call(
            &app,
            "devices.register",
            json!({"mac": "00:11:22:33:44:55", "name": "Legacy request"}),
        );
        assert_eq!(response["error"], 1);

        let response = call(
            &app,
            "devices.add_port_forward",
            json!({"uuid": "missing", "external_port": 80}),
        );
        assert_eq!(response["error"], 1);
    }
}
