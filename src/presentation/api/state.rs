use std::sync::Arc;
use serde_json::{Value, json};
use crate::application::app::App;

pub fn dispatch(app: &Arc<App>, method: &str, _request: Value) -> Result<Value, u32> {
    match method {
        "state.get" => Ok(json!(app.state())),
        _ => Err(1),
    }
}
