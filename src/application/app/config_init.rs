use crate::domain::{DesiredConfig, Lifecycle};
use crate::domain::errors::{LegacyAppError, ErrorCode, ErrorStage};
use crate::infrastructure::backend::RouterBackend;
use crate::infrastructure::storage::StateStore;
use tracing::{error, warn};

pub(crate) fn load_initial_config(
    backend: &dyn RouterBackend,
    store: &StateStore,
) -> (DesiredConfig, Lifecycle, Option<LegacyAppError>) {
    match store.load_config() {
        Ok(Some(config)) if config.schema_version == 1 => (config, Lifecycle::Booting, None),
        Ok(Some(_)) => {
            warn!("unsupported desired-state schema");
            let error = LegacyAppError::new(
                ErrorCode::StateCorrupt,
                ErrorStage::Bootstrap,
                "unsupported desired-state schema",
            );
            (DesiredConfig::empty(), Lifecycle::Degraded, Some(error))
        }
        Ok(None) => discover_default_config(backend, store),
        Err(error) => {
            error!(%error, "failed to read desired state from disk");
            (DesiredConfig::empty(), Lifecycle::Degraded, Some(error))
        }
    }
}

fn discover_default_config(
    backend: &dyn RouterBackend,
    store: &StateStore,
) -> (DesiredConfig, Lifecycle, Option<LegacyAppError>) {
    match backend.discover_primary_wifi() {
        Ok(discovered) => {
            let wan = backend
                .discover_primary_wan()
                .map_or_else(|_| crate::domain::WanDesired::default(), |w| w.to_desired());
            let config = DesiredConfig::new(discovered.to_network_config(), wan);
            if let Err(error) = store.persist_config(&config) {
                warn!(%error, "failed to persist discovered default config");
                (config, Lifecycle::Degraded, Some(error))
            } else {
                (config, Lifecycle::Booting, None)
            }
        }
        Err(error) => {
            warn!(%error, "failed to discover Wi-Fi interfaces during bootstrap");
            (DesiredConfig::empty(), Lifecycle::NeedsSetup, Some(error))
        }
    }
}
