use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    application::app::App,
    domain::device::{Device, PortForward, RegisteredDevice},
};

#[derive(Serialize)]
struct DeviceView {
    #[serde(flatten)]
    device: Device,
    #[serde(skip_serializing_if = "Option::is_none")]
    uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    is_static_ip: bool,
    port_forwards: Vec<PortForward>,
}

#[derive(Deserialize)]
struct RegisterRequest {
    mac: String,
    name: String,
}

#[derive(Deserialize)]
struct UpdateRequest {
    uuid: String,
    name: String,
    is_static_ip: bool,
}

#[derive(Deserialize)]
struct NewPortForward {
    external_port: u32,
    internal_port: u32,
    protocol: String,
}

#[derive(Deserialize)]
struct AddPortForwardRequest {
    uuid: String,
    port_forward: NewPortForward,
}

#[derive(Deserialize)]
struct RemovePortForwardRequest {
    uuid: String,
    pf_id: String,
}

#[derive(Deserialize)]
struct DeleteRequest {
    uuid: String,
}

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "devices.list" => list_devices(app),
        "devices.register" => {
            let request: RegisterRequest = serde_json::from_value(request).map_err(|_| 1_u32)?;
            app.register_device(RegisteredDevice {
                uuid: uuid::Uuid::new_v4().to_string(),
                mac: request.mac,
                name: request.name,
                is_static_ip: false,
                port_forwards: Vec::new(),
            })
            .map(|()| json!({}))
            .map_err(|_| 1)
        }
        "devices.update" => {
            let request: UpdateRequest = serde_json::from_value(request).map_err(|_| 1_u32)?;
            let mut device = registered_device(app, &request.uuid)?;
            device.name = request.name;
            device.is_static_ip = request.is_static_ip;
            app.update_device(&request.uuid, device)
                .map(|()| json!({}))
                .map_err(|_| 1)
        }
        "devices.delete" => {
            let request: DeleteRequest = serde_json::from_value(request).map_err(|_| 1_u32)?;
            app.delete_device(&request.uuid)
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
            app.add_port_forward(&request.uuid, rule)
                .map(|()| json!({}))
                .map_err(|_| 1)
        }
        "devices.remove_port_forward" => {
            let request: RemovePortForwardRequest =
                serde_json::from_value(request).map_err(|_| 1_u32)?;
            app.remove_port_forward(&request.uuid, &request.pf_id)
                .map(|()| json!({}))
                .map_err(|_| 1)
        }
        _ => Err(1),
    }
}

fn list_devices(app: &App) -> Result<Value, u32> {
    let registered = app.state().registered_devices;
    let devices = app.devices_list().map_err(|_| 1_u32)?;
    let views: Vec<DeviceView> = devices
        .into_iter()
        .map(|device| {
            let registration = registered
                .iter()
                .find(|registered| registered.mac.eq_ignore_ascii_case(&device.mac));
            DeviceView {
                device,
                uuid: registration.map(|registered| registered.uuid.clone()),
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

fn registered_device(app: &App, uuid: &str) -> Result<RegisteredDevice, u32> {
    app.state()
        .registered_devices
        .into_iter()
        .find(|device| device.uuid == uuid)
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
            json!({"mac": "00:11:22:33:44:55", "name": "Phone"}),
        );
        assert_eq!(response["error"], 0);

        let list = call(&app, "devices.list", json!({}));
        let device = list["result"]
            .as_array()
            .and_then(|devices| devices.first())
            .expect("registered device in list");
        let uuid = device["uuid"].as_str().expect("generated UUID");
        assert_eq!(device["name"], "Phone");

        let response = call(
            &app,
            "devices.update",
            json!({"uuid": uuid, "name": "Work phone", "is_static_ip": true}),
        );
        assert_eq!(response["error"], 0);

        let response = call(
            &app,
            "devices.add_port_forward",
            json!({
                "uuid": uuid,
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
            "devices.add_port_forward",
            json!({"uuid": "missing", "external_port": 80}),
        );
        assert_eq!(response["error"], 1);
    }
}
