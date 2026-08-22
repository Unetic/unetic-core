use crate::application::ddns_watcher::perform_update;
use crate::{application::App, domain::DdnsConfig};
use serde_json::{Value, json};
use std::sync::Arc;

#[repr(u32)]
pub enum DdnsError {
    InvalidConfig = 1,
    MissingCredentials = 2,
    HttpRequestFailed = 3,
    ProviderRejected = 4,
    InternalError = 5,
}

pub fn dispatch(app: &Arc<App>, method: &str, request: Value) -> Result<Value, u32> {
    match method {
        "ddns.get" => {
            let state = app.state();
            Ok(json!({
                "config": state.ddns_config,
                "status": state.ddns_status
            }))
        }
        "ddns.set" => {
            let cfg: DdnsConfig =
                serde_json::from_value(request).map_err(|_| DdnsError::InvalidConfig as u32)?;
            app.ddns_set(cfg).map_err(|_| 1u32)?;
            Ok(json!({"success": true}))
        }
        "ddns.test" => {
            let state = app.state();
            let ip = state
                .wan
                .ip_address
                .clone()
                .ok_or(DdnsError::InvalidConfig as u32)?;
            let cfg = state.ddns_config.clone();

            if !cfg.enabled {
                return Err(DdnsError::InvalidConfig as u32);
            }

            let ip_clone = ip.clone();
            let result = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| ())?;
                rt.block_on(async move { perform_update(&cfg, &ip_clone).await })
                    .map_err(|_| ())
            })
            .join()
            .map_err(|_| DdnsError::InternalError as u32)?;

            match result {
                Ok(()) => Ok(json!({"success": true, "ip": ip})),
                Err(()) => Err(DdnsError::HttpRequestFailed as u32),
            }
        }
        _ => Err(1),
    }
}
