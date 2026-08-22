use std::sync::Arc;
use serde_json::{json, Value};
use crate::application::App;

#[derive(Debug, Clone, serde::Serialize)]
#[repr(u32)]
pub enum PortsError {
    InternalError = 1,
}

pub fn dispatch(app: &Arc<App>, method: &str, _request: Value) -> Result<Value, u32> {
    match method {
        "ports.list" => app
            .ports_list()
            .map(|ports| json!(ports))
            .map_err(|_| PortsError::InternalError as u32),
        _ => Err(404),
    }
}
