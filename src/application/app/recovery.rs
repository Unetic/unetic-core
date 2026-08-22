use tracing::{info, warn};

use super::App;
use crate::{
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{
        OperationSource, OperationStatus, TransactionJournal, TransactionKind, WanDesired,
        WifiNetworkConfig,
    },
};

impl App {
    pub(crate) fn recover_from_journal(
        &self,
        journal: &TransactionJournal,
    ) -> Result<(), LegacyAppError> {
        let config = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .clone();

        match journal.kind {
            TransactionKind::Wifi if matches_old_wifi(&config, journal) => {
                return self.recover_old_wifi(journal);
            }
            TransactionKind::Wifi if matches_new_wifi(&config, journal) => {
                return self.confirm_new_wifi(journal);
            }
            TransactionKind::Wan => {
                return self.recover_wan_journal(&config, journal);
            }
            TransactionKind::Wifi => {}
        }

        warn!(
            operation_id = %journal.operation_id,
            "transaction journal does not match durable desired state; clearing journal"
        );
        self.store.clear_transaction()?;
        self.refresh_observed();
        Ok(())
    }

    fn recover_old_wifi(&self, journal: &TransactionJournal) -> Result<(), LegacyAppError> {
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
        crate::application::transaction::force_state_sync(
            self,
            &journal.targets,
            &old_wifi,
            journal.old_roaming,
            OperationSource::Recovery,
        )?;
        let error = LegacyAppError::new(
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
        Ok(())
    }

    fn confirm_new_wifi(&self, journal: &TransactionJournal) -> Result<(), LegacyAppError> {
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
        crate::application::transaction::force_state_sync(
            self,
            &journal.targets,
            &new_wifi,
            journal.new_roaming,
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
        Ok(())
    }

    fn recover_wan_journal(
        &self,
        config: &crate::domain::DesiredConfig,
        journal: &TransactionJournal,
    ) -> Result<(), LegacyAppError> {
        let (old_wan, new_wan) = wan_journal_configs(journal)?;
        if config.revision == journal.base_revision
            && crate::application::wan::wan_config_matches(&config.wan, old_wan)
        {
            return self.recover_old_wan(journal, old_wan, new_wan);
        }
        if config.revision == journal.target_revision
            && crate::application::wan::wan_config_matches(&config.wan, new_wan)
        {
            return self.confirm_new_wan(journal, new_wan);
        }

        warn!(
            operation_id = %journal.operation_id,
            "WAN transaction journal does not match durable desired state; clearing journal"
        );
        self.store.clear_transaction()?;
        self.refresh_observed();
        Ok(())
    }

    fn recover_old_wan(
        &self,
        journal: &TransactionJournal,
        old_wan: &WanDesired,
        new_wan: &WanDesired,
    ) -> Result<(), LegacyAppError> {
        info!(
            operation_id = %journal.operation_id,
            "recovering interrupted WAN transaction to old desired state"
        );
        crate::application::wan::force_wan_state_sync(
            self,
            old_wan,
            OperationSource::Recovery,
            journal.base_revision,
        )?;
        let error = interrupted_error(journal, "WAN operation was interrupted by a core restart");
        self.record_recovered_wan_operation(
            journal,
            new_wan,
            OperationStatus::Failed,
            journal.base_revision,
            Some(error),
        );
        self.store.clear_transaction()?;
        self.refresh_observed();
        Ok(())
    }

    fn confirm_new_wan(
        &self,
        journal: &TransactionJournal,
        new_wan: &WanDesired,
    ) -> Result<(), LegacyAppError> {
        info!(
            operation_id = %journal.operation_id,
            "confirming committed WAN desired state after restart"
        );
        crate::application::wan::force_wan_state_sync(
            self,
            new_wan,
            OperationSource::Recovery,
            journal.target_revision,
        )?;
        self.record_recovered_wan_operation(
            journal,
            new_wan,
            OperationStatus::Succeeded,
            journal.target_revision,
            None,
        );
        self.store.clear_transaction()?;
        self.refresh_observed();
        Ok(())
    }
}

fn matches_old_wifi(config: &crate::domain::DesiredConfig, journal: &TransactionJournal) -> bool {
    config.revision == journal.base_revision
        && config.wifi.primary.ssid == journal.old_ssid
        && config.wifi.primary.encryption == journal.old_encryption
        && config.wifi.primary.key == journal.old_key
        && config.wifi.roaming == journal.old_roaming
}

fn matches_new_wifi(config: &crate::domain::DesiredConfig, journal: &TransactionJournal) -> bool {
    config.revision == journal.target_revision
        && config.wifi.primary.ssid == journal.new_ssid
        && config.wifi.primary.encryption == journal.new_encryption
        && config.wifi.primary.key == journal.new_key
        && config.wifi.roaming == journal.new_roaming
}

fn wan_journal_configs(
    journal: &TransactionJournal,
) -> Result<(&WanDesired, &WanDesired), LegacyAppError> {
    journal
        .old_wan
        .as_ref()
        .zip(journal.new_wan.as_ref())
        .ok_or_else(|| {
            LegacyAppError::new(
                ErrorCode::StateCorrupt,
                ErrorStage::Bootstrap,
                "WAN transaction journal is missing old or new configuration",
            )
        })
}

fn interrupted_error(journal: &TransactionJournal, message: &str) -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::OperationInterrupted,
        ErrorStage::Bootstrap,
        message,
    )
    .with_operation(&journal.operation_id, Some(&journal.request_id))
}
