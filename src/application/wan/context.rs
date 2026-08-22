use crate::domain::errors::LegacyAppError;
use crate::domain::{
    OperationIntent, OperationSource, OperationStatus, PublicOperation, STATE_SCHEMA_VERSION,
    TransactionJournal, TransactionKind, WanDesired,
};

#[derive(Debug, Clone)]
pub struct WanChangeContext {
    pub operation_id: String,
    pub request_id: Option<String>,
    pub source: OperationSource,
    pub base_revision: u64,
    pub target_revision: u64,
    pub old_wan: WanDesired,
    pub new_wan: WanDesired,
}

impl WanChangeContext {
    #[must_use]
    pub fn public(
        &self,
        status: OperationStatus,
        error: Option<LegacyAppError>,
    ) -> PublicOperation {
        PublicOperation {
            id: self.operation_id.clone(),
            request_id: self.request_id.clone(),
            source: self.source,
            kind: "wan.set_config".into(),
            status,
            requested_ssid: String::new(),
            intent: Some(OperationIntent::Wan(self.new_wan.clone())),
            error,
        }
    }

    #[must_use]
    pub fn to_journal(&self, phase: OperationStatus) -> TransactionJournal {
        TransactionJournal {
            schema_version: STATE_SCHEMA_VERSION,
            operation_id: self.operation_id.clone(),
            request_id: self.request_id.clone().unwrap_or_default(),
            source: self.source,
            base_revision: self.base_revision,
            target_revision: self.target_revision,
            kind: TransactionKind::Wan,
            old_ssid: String::new(),
            new_ssid: String::new(),
            old_encryption: "none".into(),
            new_encryption: "none".into(),
            old_key: None,
            new_key: None,
            old_roaming: Default::default(),
            new_roaming: Default::default(),
            targets: Vec::new(),
            old_wan: Some(self.old_wan.clone()),
            new_wan: Some(self.new_wan.clone()),
            phase,
        }
    }
}
