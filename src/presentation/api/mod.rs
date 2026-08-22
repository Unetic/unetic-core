use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

use crate::application::app::App;

pub mod ddns;
pub mod devices;
pub mod dns;
pub mod maintenance;
pub mod ports;
pub mod state;
pub mod subscribe;
pub mod system;
pub mod wan;
pub mod wifi;

#[derive(Serialize)]
pub struct ApiEnvelope<T> {
    pub idempotence_token: String,
    pub event_seq: u64,
    pub error: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
}

pub fn dispatch(app: &Arc<App>, method: &str, request_json: &str) -> String {
    let request = match serde_json::from_str::<Value>(request_json) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => return encode_error(1, "MISSING".to_owned(), app.state().event_seq),
    };
    let idempotence_token = request
        .get("idempotence_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
    let Some(idempotence_token) = idempotence_token else {
        return encode_error(1, "MISSING".to_owned(), app.state().event_seq);
    };

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
    } else if method.starts_with("ddns.") {
        ddns::dispatch(app, method, request)
    } else if method.starts_with("mesh.") {
        mesh::dispatch(app, method, request)
    } else {
        system::dispatch(app, method, request)
    };

    let event_seq = app.state().event_seq;
    match response {
        Ok(result) => encode_ok(result, idempotence_token, event_seq),
        Err(error) => encode_error(error, idempotence_token, event_seq),
    }
}

fn encode_ok(result: Value, token: String, event_seq: u64) -> String {
    serde_json::to_string(&ApiEnvelope {
        idempotence_token: token,
        event_seq,
        error: 0,
        result: Some(result),
    })
    .unwrap_or_else(|_| r#"{"error":1}"#.into())
}

fn encode_error(error: u32, token: String, event_seq: u64) -> String {
    serde_json::to_string(&ApiEnvelope::<Value> {
        idempotence_token: token,
        event_seq,
        error,
        result: None,
    })
    .unwrap_or_else(|_| r#"{"error":1}"#.into())
}
pub mod mesh;
