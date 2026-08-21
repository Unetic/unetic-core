use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    app::App,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{API_VERSION, SetWifiConfigRequest},
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
        "devices.list" => app.devices_list().map(|devices| json!(devices)),
        "operation.get" => Ok(app.last_or_active_operation()),
        "maintenance.get" => Ok(json!(app.maintenance_get())),
        "health.get" => Ok(json!(app.health())),
        "wifi.set_config" => serde_json::from_value::<SetWifiConfigRequest>(request)
            .map_err(|error| {
                DomainError::new(
                    ErrorCode::InvalidArgument,
                    ErrorStage::Validate,
                    format!("invalid wifi.set_config request: {error}"),
                )
            })
            .and_then(|request| app.wifi_set_config(request).map(|result| json!(result))),
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
        "tools.ping" => serde_json::from_value::<crate::tools::PingRequest>(request)
            .map_err(|error| {
                DomainError::new(
                    ErrorCode::InvalidArgument,
                    ErrorStage::Validate,
                    format!("invalid tools.ping request: {error}"),
                )
            })
            .and_then(|request| app.ping(&request.host).map(|result| json!(result))),
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};

    use crate::{App, Device, MemoryBackend, StateStore, api};

    #[test]
    fn test_api_dispatch_devices_list() {
        let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
        let (tx, _rx) = mpsc::channel();
        let store = StateStore::new(std::env::temp_dir().join("unetic-test-devices-list-api"));
        let app = App::bootstrap(backend, store, tx);

        let response_str = api::dispatch(&app, "devices.list", "{}");
        let val: serde_json::Value = serde_json::from_str(&response_str).expect("valid json");
        assert_eq!(val.get("ok").and_then(|v| v.as_bool()), Some(true));

        let devices: Vec<Device> =
            serde_json::from_value(val.get("result").cloned().expect("result field"))
                .expect("valid devices array");
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].mac, "00:11:22:33:44:55");
        assert_eq!(devices[0].connection_type, "Wireless");
        assert_eq!(devices[1].mac, "66:77:88:99:aa:bb");
        assert_eq!(devices[1].connection_type, "Wired");
    }
}
