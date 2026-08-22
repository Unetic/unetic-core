use std::sync::Arc;
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
