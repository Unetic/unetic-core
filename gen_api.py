import os

base = "src/presentation/api"

mod_rs = """use std::sync::Arc;
use serde::Serialize;
use serde_json::{Value, json};

use crate::application::app::App;

pub mod maintenance;
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
    } else if method.starts_with("maintenance.") {
        maintenance::dispatch(app, method, request)
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
"""

subscribe_rs = """use std::sync::Arc;
use serde_json::{Value, json};
use crate::application::app::App;

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "state.subscribe.create" => {
            let ttl_mins = request.get("ttl_mins").and_then(Value::as_u64).unwrap_or(5) as u32;
            Ok(json!({ "subscription_id": app.subscriptions.create(ttl_mins) }))
        }
        "state.subscribe.continue" => {
            let ttl_mins = request.get("ttl_mins").and_then(Value::as_u64).unwrap_or(5) as u32;
            let sub_id = request.get("subscription_id").and_then(Value::as_str).unwrap_or("");
            app.subscriptions.continue_sub(sub_id, ttl_mins).map(|_| json!({})).map_err(|e| e as u32)
        }
        "state.subscribe.cancel" => {
            let sub_id = request.get("subscription_id").and_then(Value::as_str).unwrap_or("");
            app.subscriptions.cancel(sub_id).map(|_| json!({})).map_err(|e| e as u32)
        }
        _ => Err(1),
    }
}
"""

state_rs = """use std::sync::Arc;
use serde_json::{Value, json};
use crate::application::app::App;

pub fn dispatch(app: &Arc<App>, method: &str, _request: Value) -> Result<Value, u32> {
    match method {
        "state.get" => Ok(json!(app.state())),
        _ => Err(1),
    }
}
"""

wifi_rs = """use std::sync::Arc;
use serde_json::{Value, json};
use crate::application::app::App;
use crate::domain::SetWifiConfigRequest;

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "wifi.get" => Ok(json!(app.wifi_get())),
        "wifi.set_config" => serde_json::from_value::<SetWifiConfigRequest>(request)
            .map_err(|_| 1)
            .and_then(|request| app.set_wifi_config(request).map(|result| json!(result)).map_err(|_| 1)),
        _ => Err(1),
    }
}
"""

wan_rs = """use std::sync::Arc;
use serde_json::{Value, json};
use crate::application::app::App;
use crate::domain::SetWanRequest;

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "wan.get" => Ok(json!(app.state().wan)),
        "wan.set" | "wan.set_config" => {
            serde_json::from_value::<SetWanRequest>(request)
                .map_err(|_| 1)
                .and_then(|request| app.set_wan(request).map(|result| json!(result)).map_err(|_| 1))
        }
        _ => Err(1),
    }
}
"""

maintenance_rs = """use std::sync::Arc;
use serde_json::{Value, json};
use crate::application::app::App;

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "maintenance.get" => Ok(json!(app.maintenance_get())),
        "maintenance.enter" => {
            let reason = request
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned);
            app.maintenance_enter(reason).map(|state| json!(state)).map_err(|_| 1)
        }
        "maintenance.exit" => app.maintenance_exit().map(|state| json!(state)).map_err(|_| 1),
        _ => Err(1),
    }
}
"""

system_rs = """use std::sync::Arc;
use serde_json::{Value, json};
use crate::application::app::App;
use crate::application::tools::PingRequest;

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "switch.get" => Ok(json!(app.switch_get())),
        "system.info" => Ok(json!(app.system_info())),
        "devices.list" => app.devices_list().map(|devices| json!(devices)).map_err(|_| 1),
        "operation.get" => Ok(app.last_or_active_operation()),
        "health.get" => Ok(json!(app.health())),
        "tools.ping" => serde_json::from_value::<PingRequest>(request)
            .map_err(|_| 1)
            .and_then(|request| app.ping(&request.host).map(|result| json!(result)).map_err(|e| e as u32).map_err(|_| 1)),
        _ => Err(1),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};
    use crate::infrastructure::backend::memory::MemoryBackend;
    use crate::infrastructure::storage::StateStore;
    use crate::application::app::App;
    use crate::domain::device::Device;

    #[test]
    fn test_api_dispatch_devices_list() {
        let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
        let (tx, _rx) = mpsc::channel();
        let store = StateStore::new(std::env::temp_dir().join("unetic-test-devices-list-api-new"));
        let app = App::bootstrap(backend, store, tx);

        let response_str = crate::presentation::api::dispatch(&app, "devices.list", r#"{"idempotence_token":"xyz"}"#);
        let val: serde_json::Value = serde_json::from_str(&response_str).expect("valid json");
        assert_eq!(val.get("error").and_then(|v| v.as_u64()), Some(0));

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
"""

os.makedirs(base, exist_ok=True)
with open(f"{base}/mod.rs", "w") as f: f.write(mod_rs)
with open(f"{base}/subscribe.rs", "w") as f: f.write(subscribe_rs)
with open(f"{base}/state.rs", "w") as f: f.write(state_rs)
with open(f"{base}/wifi.rs", "w") as f: f.write(wifi_rs)
with open(f"{base}/wan.rs", "w") as f: f.write(wan_rs)
with open(f"{base}/maintenance.rs", "w") as f: f.write(maintenance_rs)
with open(f"{base}/system.rs", "w") as f: f.write(system_rs)

print("Done generating presentation/api")
