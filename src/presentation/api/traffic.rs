use std::sync::Arc;

use serde_json::{Value, json};

use crate::application::App;

pub fn dispatch(app: &Arc<App>, method: &str, _request: Value) -> Result<Value, u32> {
    match method {
        "traffic.get" => Ok(json!(app.traffic())),
        _ => Err(404),
    }
}
