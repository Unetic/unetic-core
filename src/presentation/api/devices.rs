use std::sync::Arc;
use serde_json::{Value, json};
use crate::application::app::App;
use crate::domain::device::{RegisteredDevice, PortForward};

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "devices.list" => app.devices_list().map(|devices| json!(devices)).map_err(|_| 1),
        "devices.register" => {
            serde_json::from_value::<RegisteredDevice>(request)
                .map_err(|_| 1)
                .and_then(|device| app.register_device(device).map(|_| json!({})).map_err(|_| 1))
        }
        "devices.update" => {
            // Expecting { "uuid": "...", "device": { ... } }
            #[derive(serde::Deserialize)]
            struct UpdateReq {
                uuid: String,
                device: RegisteredDevice,
            }
            serde_json::from_value::<UpdateReq>(request)
                .map_err(|_| 1)
                .and_then(|req| app.update_device(&req.uuid, req.device).map(|_| json!({})).map_err(|_| 1))
        }
        "devices.delete" => {
            #[derive(serde::Deserialize)]
            struct DeleteReq {
                uuid: String,
            }
            serde_json::from_value::<DeleteReq>(request)
                .map_err(|_| 1)
                .and_then(|req| app.delete_device(&req.uuid).map(|_| json!({})).map_err(|_| 1))
        }
        "devices.add_port_forward" => {
            #[derive(serde::Deserialize)]
            struct AddPfReq {
                uuid: String,
                port_forward: PortForward,
            }
            serde_json::from_value::<AddPfReq>(request)
                .map_err(|_| 1)
                .and_then(|req| app.add_port_forward(&req.uuid, req.port_forward).map(|_| json!({})).map_err(|_| 1))
        }
        "devices.remove_port_forward" => {
            #[derive(serde::Deserialize)]
            struct RmPfReq {
                uuid: String,
                pf_id: String,
            }
            serde_json::from_value::<RmPfReq>(request)
                .map_err(|_| 1)
                .and_then(|req| app.remove_port_forward(&req.uuid, &req.pf_id).map(|_| json!({})).map_err(|_| 1))
        }
        _ => Err(1),
    }
}
