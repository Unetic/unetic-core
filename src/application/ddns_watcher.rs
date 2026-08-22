use crate::application::app::App;
use crate::domain::{DdnsConfig, DdnsProvider, PublicState};
use crate::infrastructure::http::{update_cloudflare, update_duckdns};
use std::sync::Arc;

pub fn start_ddns_watcher(
    app: Arc<App>,
    mut event_rx: tokio::sync::broadcast::Receiver<PublicState>,
) {
    tokio::spawn(async move {
        let mut last_request: Option<(DdnsConfig, String)> = None;
        loop {
            let state = match event_rx.recv().await {
                Ok(s) => s,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };

            if !state.ddns_config.enabled {
                last_request = None;
                continue;
            }
            let current_ip = match &state.wan.ip_address {
                Some(ip) => ip.clone(),
                None => continue,
            };

            let request = (state.ddns_config.clone(), current_ip.clone());
            if last_request.as_ref() == Some(&request) {
                continue;
            }
            last_request = Some(request);

            let result = perform_update(&state.ddns_config, &current_ip).await;
            app.update_ddns_status(current_ip, result);
        }
    });
}

pub(crate) async fn perform_update(cfg: &DdnsConfig, ip: &str) -> Result<(), String> {
    match cfg.provider {
        DdnsProvider::Cloudflare => {
            update_cloudflare(
                cfg.cloudflare.as_ref().ok_or("missing cloudflare config")?,
                ip,
            )
            .await
        }
        DdnsProvider::DuckDns => {
            update_duckdns(cfg.duckdns.as_ref().ok_or("missing duckdns config")?, ip).await
        }
        DdnsProvider::None => Ok(()),
    }
}
