use crate::application::app::App;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn dispatch(app: &Arc<App>, method: &str, _request: Value) -> Result<Value, u32> {
    match method {
        "state.get" => Ok(json!(app.state())),
        _ => Err(1),
    }
}
