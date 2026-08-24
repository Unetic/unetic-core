use crate::application::App;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug, Clone, serde::Serialize)]
#[repr(u32)]
pub enum PortsError {
    InternalError = 1,
    InvalidArgument = 2,
    Unavailable = 3,
}

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "ports.list" => app
            .ports_list()
            .map(|ports| json!(ports))
            .map_err(|_| PortsError::InternalError as u32),
        "ports.switch.get" => app
            .switch_state()
            .map(|state| json!(state))
            .map_err(|_| PortsError::InternalError as u32),
        "ports.switch.hw_offload.set" => {
            let enabled = request
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or(PortsError::InvalidArgument as u32)?;
            app.set_hw_offload(enabled)
                .map(|state| json!(state))
                .map_err(|error| {
                    if error.code == crate::domain::ErrorCode::InvalidArgument {
                        PortsError::Unavailable as u32
                    } else {
                        PortsError::InternalError as u32
                    }
                })
        }
        _ => Err(404),
    }
}
