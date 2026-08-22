use std::sync::Arc;
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
