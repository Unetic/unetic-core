use std::path::Path;

use serde_json::{Map, Value, json};

use crate::domain::errors::{ErrorCode, ErrorStage, LegacyAppError};

const UBUS_STATUS_NOT_FOUND: i32 = 4;

pub fn call_ubus(object: &str, method: &str, request: Value) -> Result<Value, LegacyAppError> {
    let payload = serde_json::to_string(&request).map_err(|error| {
        LegacyAppError::new(
            ErrorCode::Internal,
            ErrorStage::Transport,
            format!("failed to encode ubus request: {error}"),
        )
    })?;

    let socket = Path::new("/var/run/ubus/ubus.sock");
    let mut connection = ubus::Connection::connect(socket).map_err(|error| {
        LegacyAppError::new(
            ErrorCode::UbusUnavailable,
            ErrorStage::Transport,
            format!("failed to connect to ubus: {error}"),
        )
        .retryable(true)
    })?;

    let response = connection
        .call(object, method, &payload)
        .map_err(|error| map_call_error(object, method, error))?;

    serde_json::from_str(&response).map_err(|error| {
        LegacyAppError::new(
            ErrorCode::UbusUnavailable,
            ErrorStage::Transport,
            format!("invalid JSON reply from {object}.{method}: {error}"),
        )
    })
}

pub fn create_rpcd_session() -> Result<String, LegacyAppError> {
    let response = call_ubus("session", "create", json!({"timeout": 300}))?;
    let sid = response
        .get("ubus_rpc_session")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            LegacyAppError::new(
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
                ["network", "write"],
                ["sqm", "read"],
                ["sqm", "write"],
                ["usteer", "read"],
                ["usteer", "write"]
            ]
        }),
    )?;
    Ok(sid)
}

pub fn destroy_rpcd_session(session: &str) -> Result<(), LegacyAppError> {
    call_ubus(
        "session",
        "destroy",
        json!({
            "ubus_rpc_session": session
        }),
    )
    .map(|_| ())
}

pub fn uci_get_config(
    config: &str,
    section: Option<&str>,
    option: Option<&str>,
    session: Option<&str>,
) -> Result<Value, LegacyAppError> {
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
        if error.code != ErrorCode::NotFound {
            return error;
        }
        LegacyAppError::new(ErrorCode::UciReadFailed, ErrorStage::Verify, error.message)
    })
}

fn map_call_error(object: &str, method: &str, error: ubus::UbusError) -> LegacyAppError {
    if matches!(error, ubus::UbusError::Status(UBUS_STATUS_NOT_FOUND)) {
        return LegacyAppError::new(
            ErrorCode::NotFound,
            ErrorStage::Transport,
            format!("ubus {object}.{method} failed: {error}"),
        );
    }

    let code = if object == "session" {
        ErrorCode::RpcdSessionLost
    } else {
        ErrorCode::UbusUnavailable
    };
    LegacyAppError::new(
        code,
        ErrorStage::Transport,
        format!("ubus {object}.{method} failed: {error}"),
    )
    .retryable(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_ubus_not_found_for_optional_uci_sections() {
        let error = map_call_error("uci", "get", ubus::UbusError::Status(4));

        assert_eq!(error.code, ErrorCode::NotFound);
        assert!(!error.retryable);
    }

    #[test]
    fn transport_failures_are_not_reported_as_missing_sections() {
        let error = map_call_error("uci", "get", ubus::UbusError::InvalidData("invalid reply"));

        assert_eq!(error.code, ErrorCode::UbusUnavailable);
        assert!(error.retryable);
    }
}
