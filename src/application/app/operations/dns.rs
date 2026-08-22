use std::net::IpAddr;

use crate::{
    application::app::App,
    domain::{
        DnsConfig, DnsRecord,
        errors::{ErrorCode, ErrorStage, LegacyAppError},
    },
};

impl App {
    pub fn dns_set(&self, config: DnsConfig) -> Result<(), LegacyAppError> {
        validate_dns_config(&config)?;
        self.replace_dns_config(config)
    }

    pub fn dns_add_record(&self, record: DnsRecord) -> Result<(), LegacyAppError> {
        validate_record(&record)?;
        let mut config = self.state().dns;
        if config.custom_records.iter().any(|current| {
            current.id == record.id || current.hostname.eq_ignore_ascii_case(&record.hostname)
        }) {
            return Err(invalid_argument("duplicate DNS record"));
        }
        config.custom_records.push(record);
        self.replace_dns_config(config)
    }

    pub fn dns_remove_record(&self, id: &str) -> Result<(), LegacyAppError> {
        validate_identifier(id)?;
        let mut config = self.state().dns;
        let original_len = config.custom_records.len();
        config.custom_records.retain(|record| record.id != id);
        if config.custom_records.len() == original_len {
            return Err(LegacyAppError::new(
                ErrorCode::NotFound,
                ErrorStage::Validate,
                "DNS record not found",
            ));
        }
        self.replace_dns_config(config)
    }

    fn replace_dns_config(&self, new_dns: DnsConfig) -> Result<(), LegacyAppError> {
        let old_config = {
            let inner = self.inner.lock().expect("app state poisoned");
            inner.config.clone()
        };
        let mut new_config = old_config.clone();
        new_config.dns = new_dns.clone();
        new_config.revision = new_config.revision.saturating_add(1);

        self.store.persist_config(&new_config)?;
        if let Err(error) = self.backend.write_dns_config(&new_dns) {
            let _ = self.store.persist_config(&old_config);
            let _ = self.backend.write_dns_config(&old_config.dns);
            return Err(error);
        }

        self.inner.lock().expect("app state poisoned").config = new_config;
        self.publish();
        Ok(())
    }
}

fn validate_dns_config(config: &DnsConfig) -> Result<(), LegacyAppError> {
    if config
        .upstream
        .iter()
        .any(|ip| ip.parse::<IpAddr>().is_err())
    {
        return Err(invalid_argument("invalid upstream IP address"));
    }
    let dhcp_end = config.dhcp_start.saturating_add(config.dhcp_limit);
    if config.dhcp_start == 0 || config.dhcp_limit == 0 || dhcp_end > 255 {
        return Err(invalid_argument("invalid DHCP address range"));
    }
    if config.dhcp_lease_hours == 0 {
        return Err(invalid_argument("DHCP lease duration must be positive"));
    }
    if let Some(domain) = &config.local_domain
        && !is_valid_hostname(domain)
    {
        return Err(invalid_argument("invalid local domain"));
    }
    for record in &config.custom_records {
        validate_record(record)?;
    }
    Ok(())
}

fn validate_record(record: &DnsRecord) -> Result<(), LegacyAppError> {
    validate_identifier(&record.id)?;
    if record.ip.parse::<IpAddr>().is_err() {
        return Err(invalid_argument("invalid DNS record IP address"));
    }
    if !is_valid_hostname(&record.hostname) {
        return Err(invalid_argument("invalid DNS record hostname"));
    }
    Ok(())
}

fn validate_identifier(value: &str) -> Result<(), LegacyAppError> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(invalid_argument("invalid DNS record ID"))
    }
}

fn is_valid_hostname(hostname: &str) -> bool {
    !hostname.is_empty()
        && hostname.len() <= 253
        && hostname.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn invalid_argument(message: impl Into<String>) -> LegacyAppError {
    LegacyAppError::new(ErrorCode::InvalidArgument, ErrorStage::Validate, message)
}

#[cfg(test)]
mod tests {
    use super::is_valid_hostname;

    #[test]
    fn validates_each_hostname_label() {
        assert!(is_valid_hostname("printer.home"));
        assert!(!is_valid_hostname("-printer.home"));
        assert!(!is_valid_hostname("printer..home"));
    }
}
