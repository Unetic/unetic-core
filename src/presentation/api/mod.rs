use std::sync::Arc;
use serde::Serialize;
use serde_json::Value;

use crate::application::app::App;

pub mod maintenance;
pub mod ports;
pub mod state;
pub mod subscribe;
pub mod system;
pub mod wan;
pub mod wifi;
pub mod devices;
pub mod dns;

#[derive(Serialize)]
pub struct ApiEnvelope<T> {
    pub idempotence_token: String,
    pub event_seq: u64,
    pub error: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
}

pub fn dispatch(app: &Arc<App>, method: &str, request_json: &str) -> String {
    let request_val: Result<Value, _> = serde_json::from_str(request_json);
    let mut idempotence_token = String::new();
    let mut request = Value::Null;

    if let Ok(Value::Object(ref map)) = request_val {
        if let Some(Value::String(t)) = map.get("idempotence_token") {
            if !t.is_empty() {
                idempotence_token = t.clone();
            }
        }
        request = request_val.unwrap();
    }

    if idempotence_token.is_empty() {
        let envelope = ApiEnvelope::<Value> {
            idempotence_token: "MISSING".to_string(),
            event_seq: 0,
            error: 1,
            result: None,
        };
        return serde_json::to_string(&envelope).unwrap();
    }

    let response = if method.starts_with("state.subscribe.") {
        subscribe::dispatch(app, method, request)
    } else if method.starts_with("state.") {
        state::dispatch(app, method, request)
    } else if method.starts_with("wifi.") {
        wifi::dispatch(app, method, request)
    } else if method.starts_with("wan.") {
        wan::dispatch(app, method, request)
    } else if method.starts_with("ports.") {
        ports::dispatch(app, method, request)
    } else if method.starts_with("maintenance.") {
        maintenance::dispatch(app, method, request)
    } else if method.starts_with("devices.") {
        devices::dispatch(app, method, request)
    } else if method.starts_with("dns.") {
        dns::dispatch(app, method, request)
    } else {
        system::dispatch(app, method, request)
    };

    match response {
        Ok(result) => encode_ok(result, idempotence_token),
        Err(error) => encode_error(error, idempotence_token),
    }
}

pub fn encode_ok(result: Value, token: String) -> String {
    serde_json::to_string(&ApiEnvelope {
        idempotence_token: token,
        event_seq: 0,
        error: 0,
        result: Some(result),
    })
    .unwrap_or_else(|_| r#"{"error":1}"#.into())
}

pub fn encode_error(error: u32, token: String) -> String {
    serde_json::to_string(&ApiEnvelope::<Value> {
        idempotence_token: token,
        event_seq: 0,
        error,
        result: None,
    })
    .unwrap_or_else(|_| r#"{"error":1}"#.into())
}
