use std::net::IpAddr;
use crate::application::app::App;
use crate::domain::errors::{LegacyAppError, ErrorCode, ErrorStage};
use crate::domain::{DnsConfig, DnsRecord};

impl App {
    pub fn dns_set(&self, cfg: DnsConfig) -> Result<(), LegacyAppError> {
        for ip in &cfg.upstream {
            if ip.parse::<IpAddr>().is_err() {
                return Err(LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, "Invalid upstream IP"));
            }
        }
        
        if cfg.dhcp_start == 0 || cfg.dhcp_limit == 0 {
            return Err(LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, "Invalid DHCP range"));
        }

        let mut inner = self.inner.lock().expect("app state poisoned");
        inner.config.dns = cfg.clone();
        inner.config.revision += 1;
        self.store.persist_config(&inner.config)?;
        drop(inner);
        
        self.publish();
        
        if let Err(e) = self.backend.write_dns_config(&cfg) {
            return Err(LegacyAppError::new(ErrorCode::UciApplyFailed, ErrorStage::Apply, e.to_string()));
        }
        
        Ok(())
    }

    pub fn dns_add_record(&self, record: DnsRecord) -> Result<(), LegacyAppError> {
        if record.ip.parse::<IpAddr>().is_err() {
            return Err(LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, "Invalid IP"));
        }
        
        if record.hostname.is_empty() || !record.hostname.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.') {
            return Err(LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, "Invalid hostname"));
        }
        
        let mut inner = self.inner.lock().expect("app state poisoned");
        
        if inner.config.dns.custom_records.iter().any(|r| r.id == record.id || r.hostname == record.hostname) {
            return Err(LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, "Duplicate record"));
        }
        
        inner.config.dns.custom_records.push(record);
        let cfg = inner.config.dns.clone();
        inner.config.revision += 1;
        self.store.persist_config(&inner.config)?;
        drop(inner);
        
        self.publish();
        
        if let Err(e) = self.backend.write_dns_config(&cfg) {
            return Err(LegacyAppError::new(ErrorCode::UciApplyFailed, ErrorStage::Apply, e.to_string()));
        }
        
        Ok(())
    }

    pub fn dns_remove_record(&self, id: &str) -> Result<(), LegacyAppError> {
        let mut inner = self.inner.lock().expect("app state poisoned");
        
        let initial_len = inner.config.dns.custom_records.len();
        inner.config.dns.custom_records.retain(|r| r.id != id);
        
        if inner.config.dns.custom_records.len() == initial_len {
            return Err(LegacyAppError::new(ErrorCode::NotFound, ErrorStage::Validate, "Record not found"));
        }
        
        let cfg = inner.config.dns.clone();
        inner.config.revision += 1;
        self.store.persist_config(&inner.config)?;
        drop(inner);
        
        self.publish();
        
        if let Err(e) = self.backend.write_dns_config(&cfg) {
            return Err(LegacyAppError::new(ErrorCode::UciApplyFailed, ErrorStage::Apply, e.to_string()));
        }
        
        Ok(())
    }
}
