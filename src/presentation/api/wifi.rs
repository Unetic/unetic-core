use crate::application::app::App;
use crate::domain::SetWifiConfigRequest;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "wifi.get" => Ok(json!(app.wifi_get())),
        "wifi.set_config" => serde_json::from_value::<SetWifiConfigRequest>(request)
            .map_err(|_| 1)
            .and_then(|request| {
                app.set_wifi_config(request)
                    .map(|result| json!(result))
                    .map_err(|_| 1)
            }),
        _ => Err(1),
    }
}
