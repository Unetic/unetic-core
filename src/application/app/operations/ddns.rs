use crate::{
    application::app::{App, StateTopic},
    domain::{
        DdnsConfig, DdnsProvider, DdnsStatus,
        errors::{ErrorCode, ErrorStage, LegacyAppError},
    },
};

fn now_unix_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl App {
    pub fn ddns_set(&self, config: DdnsConfig) -> Result<(), LegacyAppError> {
        validate_ddns_config(&config)?;

        let mut inner = self.inner.lock().expect("app state poisoned");
        let mut desired = inner.config.clone();
        desired.ddns = config;
        desired.revision = desired.revision.saturating_add(1);
        self.store.persist_config(&desired)?;
        inner.config = desired;
        drop(inner);
        self.publish(StateTopic::Ddns);
        Ok(())
    }

    pub(crate) fn update_ddns_status(&self, ip: String, result: Result<(), String>) {
        let status = DdnsStatus {
            last_ip: Some(ip),
            last_update_ts: Some(now_unix_ts()),
            last_error: result.err(),
        };
        let mut inner = self.inner.lock().expect("app state poisoned");
        inner.ddns_status = status;
        drop(inner);
        self.publish(StateTopic::Ddns);
    }
}

fn validate_ddns_config(config: &DdnsConfig) -> Result<(), LegacyAppError> {
    if !config.enabled {
        return Ok(());
    }

    let valid = match config.provider {
        DdnsProvider::Cloudflare => config.cloudflare.as_ref().is_some_and(|cloudflare| {
            all_present([
                &cloudflare.zone_id,
                &cloudflare.record_id,
                &cloudflare.api_token,
                &cloudflare.hostname,
            ])
        }),
        DdnsProvider::DuckDns => config
            .duckdns
            .as_ref()
            .is_some_and(|duckdns| all_present([&duckdns.token, &duckdns.domain])),
        DdnsProvider::None => false,
    };
    if valid {
        Ok(())
    } else {
        Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "enabled DDNS provider configuration is incomplete",
        ))
    }
}

fn all_present<const N: usize>(values: [&String; N]) -> bool {
    values.iter().all(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::validate_ddns_config;
    use crate::domain::{DdnsConfig, DdnsProvider};

    #[test]
    fn rejects_enabled_ddns_without_provider() {
        let config = DdnsConfig {
            enabled: true,
            provider: DdnsProvider::None,
            ..DdnsConfig::default()
        };

        assert!(validate_ddns_config(&config).is_err());
    }
}
