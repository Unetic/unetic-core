use tracing::{info, warn};

use super::App;
use crate::{
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{OperationSource, OperationStatus, TransactionJournal, WifiNetworkConfig},
    transaction,
};

impl App {
    pub(crate) fn recover_from_journal(
        &self,
        journal: &TransactionJournal,
    ) -> Result<(), DomainError> {
        let config = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .clone();

        let old_matches = config.revision == journal.base_revision
            && config.wifi.primary.ssid == journal.old_ssid
            && config.wifi.primary.encryption == journal.old_encryption
            && config.wifi.primary.key == journal.old_key;

        if old_matches {
            info!(
                operation_id = %journal.operation_id,
                "recovering interrupted transaction to old desired state"
            );
            let old_wifi = WifiNetworkConfig {
                ssid: journal.old_ssid.clone(),
                encryption: journal.old_encryption.clone(),
                key: journal.old_key.clone(),
                targets: journal.targets.clone(),
            };
            transaction::force_state_sync(
                self,
                &journal.targets,
                &old_wifi,
                OperationSource::Recovery,
            )?;
            let error = DomainError::new(
                ErrorCode::OperationInterrupted,
                ErrorStage::Bootstrap,
                "operation was interrupted by a core restart and rolled back",
            )
            .with_operation(&journal.operation_id, Some(&journal.request_id));
            self.record_recovered_operation(
                journal,
                OperationStatus::Failed,
                journal.base_revision,
                Some(error),
            );
            self.store.clear_transaction()?;
            self.refresh_observed();
            return Ok(());
        }

        let new_matches = config.revision == journal.target_revision
            && config.wifi.primary.ssid == journal.new_ssid
            && config.wifi.primary.encryption == journal.new_encryption
            && config.wifi.primary.key == journal.new_key;

        if new_matches {
            info!(
                operation_id = %journal.operation_id,
                "confirming committed desired state after restart"
            );
            let new_wifi = WifiNetworkConfig {
                ssid: journal.new_ssid.clone(),
                encryption: journal.new_encryption.clone(),
                key: journal.new_key.clone(),
                targets: journal.targets.clone(),
            };
            transaction::force_state_sync(
                self,
                &journal.targets,
                &new_wifi,
                OperationSource::Recovery,
            )?;
            self.record_recovered_operation(
                journal,
                OperationStatus::Succeeded,
                journal.target_revision,
                None,
            );
            self.store.clear_transaction()?;
            self.refresh_observed();
            return Ok(());
        }

        warn!(
            operation_id = %journal.operation_id,
            "transaction journal does not match durable desired state; clearing journal"
        );
        self.store.clear_transaction()?;
        self.refresh_observed();
        Ok(())
    }
}
