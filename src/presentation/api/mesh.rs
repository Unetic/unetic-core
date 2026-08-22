use std::sync::Arc;
use serde_json::{Value, json};
use crate::application::app::App;
use serde::Deserialize;

#[derive(Deserialize)]
struct MacRequest {
    mac: String,
}

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "mesh.pair_accept" => {
            let req: MacRequest = serde_json::from_value(request).map_err(|_| 1u32)?;
            app.mesh_pair_accept(req.mac).map(|_| json!({})).map_err(|_| 1u32)
        }
        "mesh.pair_reject" => {
            let req: MacRequest = serde_json::from_value(request).map_err(|_| 1u32)?;
            app.mesh_pair_reject(req.mac);
            Ok(json!({}))
        }
        _ => Err(1),
    }
}
