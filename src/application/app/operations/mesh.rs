use crate::{
    application::{app::App, transaction::force_state_sync},
    domain::{
        OperationSource, WifiNetworkConfig,
        errors::{ErrorCode, ErrorStage, LegacyAppError},
        extender::KnownExtender,
    },
};

impl App {
    pub fn mesh_pair_accept(&self, mac: String) -> Result<(), LegacyAppError> {
        let mac = mac.to_ascii_lowercase();
        let mut inner = self.inner.lock().expect("app state poisoned");
        let pending = inner
            .pending_extenders
            .iter()
            .find(|extender| extender.mac.eq_ignore_ascii_case(&mac))
            .cloned()
            .ok_or_else(|| not_found("pending extender"))?;

        let token = uuid::Uuid::new_v4().to_string();
        let known = KnownExtender {
            mac: mac.clone(),
            model: pending.model,
            ip: String::new(),
            auth_token: token,
        };
        let mut config = inner.config.clone();
        config.extenders.retain(|extender| extender.mac != mac);
        config.extenders.push(known);
        config.revision = config.revision.saturating_add(1);
        self.store.persist_config(&config)?;

        inner.config = config;
        inner
            .pending_extenders
            .retain(|extender| !extender.mac.eq_ignore_ascii_case(&mac));
        inner.approved_pairings.insert(mac, pending.pairing_key);
        drop(inner);
        self.publish();
        Ok(())
    }

    pub fn mesh_pair_reject(&self, mac: String) -> Result<(), LegacyAppError> {
        let mut inner = self.inner.lock().expect("app state poisoned");
        let original_len = inner.pending_extenders.len();
        inner
            .pending_extenders
            .retain(|extender| !extender.mac.eq_ignore_ascii_case(&mac));
        if inner.pending_extenders.len() == original_len {
            return Err(not_found("pending extender"));
        }
        drop(inner);
        self.publish();
        Ok(())
    }

    pub(crate) fn apply_master_wifi_config(
        &self,
        mut config: WifiNetworkConfig,
    ) -> Result<(), LegacyAppError> {
        let current = {
            let inner = self.inner.lock().expect("app state poisoned");
            inner.config.wifi.primary.clone()
        };
        config.targets = current.targets.clone();
        if config == current {
            return Ok(());
        }

        force_state_sync(self, &config.targets, &config, OperationSource::Reconcile)?;

        let mut inner = self.inner.lock().expect("app state poisoned");
        let mut desired = inner.config.clone();
        desired.wifi.primary = config;
        desired.revision = desired.revision.saturating_add(1);
        self.store.persist_config(&desired)?;
        inner.config = desired;
        drop(inner);
        self.refresh_observed();
        self.publish();
        Ok(())
    }
}

fn not_found(entity: &str) -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::NotFound,
        ErrorStage::Validate,
        format!("{entity} not found"),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{MemoryBackend, StateStore, domain::extender::PendingExtender};

    #[test]
    fn pairing_token_requires_matching_key_and_is_single_use() {
        let backend = Arc::new(MemoryBackend::new("Home", &["radio0"]));
        let (events, _) = tokio::sync::broadcast::channel(16);
        let state_dir = std::env::temp_dir().join(format!("unetic-mesh-{}", uuid::Uuid::new_v4()));
        let app = App::bootstrap(backend, StateStore::new(state_dir), events);
        app.mesh_add_pending(PendingExtender {
            mac: "aa:bb:cc:dd:ee:ff".to_owned(),
            model: "Extender".to_owned(),
            pairing_key: "secret".to_owned(),
        });
        app.mesh_pair_accept("AA:BB:CC:DD:EE:FF".to_owned())
            .expect("pairing accepted");

        assert!(
            app.take_approved_pairing_token("aa:bb:cc:dd:ee:ff", "wrong")
                .is_none()
        );
        assert!(
            app.take_approved_pairing_token("aa:bb:cc:dd:ee:ff", "secret")
                .is_some()
        );
        assert!(
            app.take_approved_pairing_token("aa:bb:cc:dd:ee:ff", "secret")
                .is_none()
        );
    }
}
