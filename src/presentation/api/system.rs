use crate::application::app::App;
use crate::application::tools::PingRequest;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "system.info" => Ok(json!(app.system_info())),
        "operation.get" => Ok(app.last_or_active_operation()),
        "health.get" => Ok(json!(app.health())),
        "tools.ping" => serde_json::from_value::<PingRequest>(request)
            .map_err(|_| 1)
            .and_then(|request| {
                app.ping(&request.host)
                    .map(|result| json!(result))
                    .map_err(|e| e as u32)
                    .map_err(|_| 1)
            }),
        _ => Err(1),
    }
}

#[cfg(test)]
mod tests {
    use crate::application::app::App;
    use crate::domain::device::Device;
    use crate::infrastructure::backend::memory::MemoryBackend;
    use crate::infrastructure::storage::StateStore;
    use std::sync::Arc;

    #[test]
    fn test_api_dispatch_devices_list() {
        let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let store = StateStore::new(std::env::temp_dir().join("unetic-test-devices-list-api-new"));
        let app = App::bootstrap(backend, store, tx);

        let response_str = crate::presentation::api::dispatch(
            &app,
            "devices.list",
            r#"{"idempotence_token":"xyz"}"#,
        );
        let val: serde_json::Value = serde_json::from_str(&response_str).expect("valid json");
        assert_eq!(val.get("error").and_then(|v| v.as_u64()), Some(0));

        let devices: Vec<Device> =
            serde_json::from_value(val.get("result").cloned().expect("result field"))
                .expect("valid devices array");
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].mac, "00:11:22:33:44:55");
        assert_eq!(
            devices[0].connection,
            crate::domain::device::DeviceConnection::Wireless {
                signal_dbm: -82,
                interface: "wlan0".into(),
                network: Some("Unetic".into()),
            }
        );
        assert_eq!(devices[1].mac, "66:77:88:99:aa:bb");
        assert_eq!(
            devices[1].connection,
            crate::domain::device::DeviceConnection::Wired {
                port_id: "lan1".into(),
            }
        );
    }
}
