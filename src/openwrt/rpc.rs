use std::path::Path;

use serde_json::{Map, Value, json};

use crate::errors::{DomainError, ErrorCode, ErrorStage};

pub fn call_ubus(object: &str, method: &str, request: Value) -> Result<Value, DomainError> {
    let payload = serde_json::to_string(&request).map_err(|error| {
        DomainError::new(
            ErrorCode::Internal,
            ErrorStage::Transport,
            format!("failed to encode ubus request: {error}"),
        )
    })?;

    let socket = Path::new("/var/run/ubus/ubus.sock");
    let mut connection = ubus::Connection::connect(socket).map_err(|error| {
        DomainError::new(
            ErrorCode::UbusUnavailable,
            ErrorStage::Transport,
            format!("failed to connect to ubus: {error}"),
        )
        .retryable(true)
    })?;

    let response = connection.call(object, method, &payload).map_err(|error| {
        let code = if object == "session" {
            ErrorCode::RpcdSessionLost
        } else {
            ErrorCode::UbusUnavailable
        };
        DomainError::new(
            code,
            ErrorStage::Transport,
            format!("ubus {object}.{method} failed: {error}"),
        )
        .retryable(true)
    })?;

    serde_json::from_str(&response).map_err(|error| {
        DomainError::new(
            ErrorCode::UbusUnavailable,
            ErrorStage::Transport,
            format!("invalid JSON reply from {object}.{method}: {error}"),
        )
    })
}

pub fn create_rpcd_session() -> Result<String, DomainError> {
    let response = call_ubus("session", "create", json!({"timeout": 300}))?;
    let sid = response
        .get("ubus_rpc_session")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            DomainError::new(
                ErrorCode::RpcdSessionLost,
                ErrorStage::Transport,
                "rpcd session.create did not return ubus_rpc_session",
            )
        })?
        .to_owned();

    call_ubus(
        "session",
        "grant",
        json!({
            "ubus_rpc_session": sid,
            "scope": "uci",
            "objects": [
                ["wireless", "read"],
                ["wireless", "write"],
                ["network", "read"],
                ["network", "write"]
            ]
        }),
    )?;
    Ok(sid)
}

pub fn uci_get_config(
    config: &str,
    section: Option<&str>,
    option: Option<&str>,
    session: Option<&str>,
) -> Result<Value, DomainError> {
    let mut request = Map::new();
    request.insert("config".into(), Value::String(config.into()));
    if let Some(section) = section {
        request.insert("section".into(), Value::String(section.into()));
    }
    if let Some(option) = option {
        request.insert("option".into(), Value::String(option.into()));
    }
    if let Some(session) = session {
        request.insert("ubus_rpc_session".into(), Value::String(session.to_owned()));
    }
    call_ubus("uci", "get", Value::Object(request)).map_err(|error| {
        DomainError::new(ErrorCode::UciReadFailed, ErrorStage::Verify, error.message)
            .retryable(error.retryable)
    })
}
