use crate::application::app::App;
use crate::domain::SetWanRequest;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "wan.get" => Ok(json!(app.state().wan)),
        "wan.get_config" => Ok(json!(app.wan_config())),
        "wan.set" | "wan.set_config" => serde_json::from_value::<SetWanRequest>(request)
            .map_err(|_| 1)
            .and_then(|request| {
                app.set_wan(request)
                    .map(|result| json!(result))
                    .map_err(|_| 1)
            }),
        _ => Err(1),
    }
}
