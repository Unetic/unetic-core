use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    app::App,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{API_VERSION, SetSsidRequest},
};

#[derive(Serialize)]
struct ApiEnvelope<T: Serialize> {
    api_version: u32,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<DomainError>,
    state: crate::model::PublicState,
}

pub fn dispatch(app: &Arc<App>, method: &str, request_json: &str) -> String {
    let request = match serde_json::from_str::<Value>(request_json) {
        Ok(request) => request,
        Err(error) => {
            return encode_error(
                app,
                DomainError::new(
                    ErrorCode::InvalidArgument,
                    ErrorStage::Validate,
                    format!("invalid JSON request: {error}"),
                ),
            );
        }
    };

    let response = match method {
        "state" => Ok(json!(app.state())),
        "wifi.get" => Ok(json!(app.wifi_get())),
        "wan.get" => Ok(json!(app.state().wan)),
        "switch.get" => Ok(json!(app.switch_get())),
        "system.info" => Ok(json!(app.system_info())),
        "operation.get" => Ok(app.last_or_active_operation()),
        "maintenance.get" => Ok(json!(app.maintenance_get())),
        "health.get" => Ok(json!(app.health())),
        "wifi.set_ssid" => serde_json::from_value::<SetSsidRequest>(request)
            .map_err(|error| {
                DomainError::new(
                    ErrorCode::InvalidArgument,
                    ErrorStage::Validate,
                    format!("invalid wifi.set_ssid request: {error}"),
                )
            })
            .and_then(|request| app.set_ssid(request).map(|result| json!(result))),
        "wan.set" | "wan.set_config" => {
            serde_json::from_value::<crate::model::SetWanRequest>(request)
                .map_err(|error| {
                    DomainError::new(
                        ErrorCode::InvalidArgument,
                        ErrorStage::Validate,
                        format!("invalid wan.set request: {error}"),
                    )
                })
                .and_then(|request| app.set_wan(request).map(|result| json!(result)))
        }
        "maintenance.enter" => {
            let reason = request
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned);
            app.maintenance_enter(reason).map(|state| json!(state))
        }
        "maintenance.exit" => app.maintenance_exit().map(|state| json!(state)),
        _ => Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            format!("unknown Unetic method: {method}"),
        )),
    };

    match response {
        Ok(result) => encode_ok(app, result),
        Err(error) => encode_error(app, error),
    }
}

fn encode_ok(app: &App, result: Value) -> String {
    serde_json::to_string(&ApiEnvelope {
        api_version: API_VERSION,
        ok: true,
        result: Some(result),
        error: None,
        state: app.state(),
    })
    .unwrap_or_else(|_| r#"{"api_version":1,"ok":false}"#.into())
}

fn encode_error(app: &App, error: DomainError) -> String {
    serde_json::to_string(&ApiEnvelope::<Value> {
        api_version: API_VERSION,
        ok: false,
        result: None,
        error: Some(error),
        state: app.state(),
    })
    .unwrap_or_else(|_| r#"{"api_version":1,"ok":false}"#.into())
}
