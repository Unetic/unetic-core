use crate::{
    application::app::App,
    domain::{DdnsConfig, DdnsProvider, DdnsStatus},
    domain::errors::{LegacyAppError, ErrorCode, ErrorStage},
};

fn now_unix_ts() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

impl App {
    pub fn ddns_set(&self, cfg: DdnsConfig) -> Result<(), LegacyAppError> {
        // Validate required fields
        match cfg.provider {
            DdnsProvider::Cloudflare => {
                if cfg.cloudflare.is_none() {
                    return Err(LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, "cloudflare config missing"));
                }
            }
            DdnsProvider::DuckDns => {
                if cfg.duckdns.is_none() {
                    return Err(LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, "duckdns config missing"));
                }
            }
            DdnsProvider::None => {}
        }

        let mut inner = self.inner.lock().unwrap();
        inner.config.ddns = cfg;
        inner.config.revision += 1;
        let config_clone = inner.config.clone();
        
        if let Err(e) = self.store.persist_config(&config_clone) {
            return Err(e);
        }
        
        drop(inner);
        self.publish();
        Ok(())
    }

    pub(crate) fn update_ddns_status(&self, ip: String, result: Result<(), String>) {
        let status = DdnsStatus {
            last_ip: Some(ip),
            last_update_ts: Some(now_unix_ts()),
            last_error: result.err(),
        };
        let mut inner = self.inner.lock().unwrap();
        inner.ddns_status = status;
        drop(inner);
        self.publish();
    }
}
