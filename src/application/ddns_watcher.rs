use std::sync::Arc;
use crate::application::app::App;
use crate::domain::{PublicState, DdnsConfig, DdnsProvider};
use crate::infrastructure::http::{update_cloudflare, update_duckdns};

pub fn start_ddns_watcher(app: Arc<App>, mut event_rx: tokio::sync::broadcast::Receiver<PublicState>) {
    tokio::spawn(async move {
        let mut last_ip: Option<String> = None;
        loop {
            let state = match event_rx.recv().await {
                Ok(s) => s,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            
            if !state.ddns_config.enabled { continue; }
            let current_ip = match &state.wan.ip_address { Some(ip) => ip.clone(), None => continue };
            
            if Some(&current_ip) == last_ip.as_ref() { continue; }
            last_ip = Some(current_ip.clone());
            
            let cfg = state.ddns_config.clone();
            let app2 = Arc::clone(&app);
            
            tokio::spawn(async move {
                let result = perform_update(&cfg, &current_ip).await;
                app2.update_ddns_status(current_ip, result);
            });
        }
    });
}

pub(crate) async fn perform_update(cfg: &DdnsConfig, ip: &str) -> Result<(), String> {
    match cfg.provider {
        DdnsProvider::Cloudflare => update_cloudflare(cfg.cloudflare.as_ref().ok_or("missing cloudflare config")?, ip).await,
        DdnsProvider::DuckDns => update_duckdns(cfg.duckdns.as_ref().ok_or("missing duckdns config")?, ip).await,
        DdnsProvider::None => Ok(()),
    }
}
