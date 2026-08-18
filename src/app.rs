use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use tracing::{error, info, warn};

use crate::{
    backend::RouterBackend,
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{
        API_VERSION, DesiredConfig, DriftState, HealthState, LastOperation, Lifecycle,
        MaintenanceState, OperationAccepted, OperationSource, OperationStatus, PublicOperation,
        PublicState, SetSsidRequest, TransactionJournal, WifiPublicState, WifiStatus,
    },
    storage::StateStore,
    transaction::{self, ChangeContext},
};

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub reconcile_interval: Duration,
    pub verify_timeout: Duration,
    pub verify_sample_delay: Duration,
    pub rollback_verify_timeout: Duration,
    pub rpcd_rollback_timeout_secs: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            reconcile_interval: Duration::from_secs(2),
            verify_timeout: Duration::from_secs(90),
            verify_sample_delay: Duration::from_millis(400),
            rollback_verify_timeout: Duration::from_secs(10),
            rpcd_rollback_timeout_secs: 120,
        }
    }
}

pub(crate) struct Inner {
    pub config: DesiredConfig,
    pub lifecycle: Lifecycle,
    pub maintenance: bool,
    pub maintenance_exiting: bool,
    pub maintenance_reason: Option<String>,
    pub observed: BTreeMap<String, String>,
    pub runtime_healthy: bool,
    pub active_operation: Option<PublicOperation>,
    pub last_user_operation: Option<LastOperation>,
    pub last_system_error: Option<DomainError>,
    pub event_seq: u64,
    pub boot_id: String,
    pub health: HealthState,
    pub repair_failures: u8,
}

pub struct App {
    pub(crate) backend: Arc<dyn RouterBackend>,
    pub(crate) store: StateStore,
    pub(crate) inner: Mutex<Inner>,
    pub(crate) event_tx: Sender<PublicState>,
    pub(crate) shutdown: AtomicBool,
    pub(crate) op_counter: AtomicU64,
    pub(crate) timing: Timing,
}

impl App {
    pub fn bootstrap(
        backend: Arc<dyn RouterBackend>,
        store: StateStore,
        event_tx: Sender<PublicState>,
    ) -> Arc<Self> {
        Self::bootstrap_with_timing(backend, store, event_tx, Timing::default())
    }

    pub fn bootstrap_with_timing(
        backend: Arc<dyn RouterBackend>,
        store: StateStore,
        event_tx: Sender<PublicState>,
        timing: Timing,
    ) -> Arc<Self> {
        let store_ready = store.ensure();
        let mut startup_error = store_ready.as_ref().err().cloned();
        let boot_id = generate_id("boot");
        let last = store.load_last_operation().unwrap_or_else(|error| {
            warn!(%error, "failed to load last operation");
            None
        });

        let (config, lifecycle) = match store.load_config() {
            Ok(Some(config)) if config.schema_version == 1 => (config, Lifecycle::Booting),
            Ok(Some(_)) => {
                warn!("unsupported desired-state schema");
                startup_error = Some(DomainError::new(
                    ErrorCode::StateCorrupt,
                    ErrorStage::Bootstrap,
                    "unsupported desired-state schema",
                ));
                (DesiredConfig::empty(), Lifecycle::Degraded)
            }
            Ok(None) => match backend.discover_primary_wifi() {
                Ok(discovered) => {
                    let config = DesiredConfig::new(discovered.ssid, discovered.targets);
                    match store.persist_config(&config) {
                        Ok(()) => (config, Lifecycle::Booting),
                        Err(error) => {
                            error!(%error, "failed to persist bootstrap config");
                            startup_error = Some(error);
                            (config, Lifecycle::Degraded)
                        }
                    }
                }
                Err(error) => {
                    warn!(%error, "could not bootstrap primary Wi-Fi");
                    startup_error = Some(error);
                    (DesiredConfig::empty(), Lifecycle::NeedsSetup)
                }
            },
            Err(error) => {
                error!(%error, "failed to load desired state");
                startup_error = Some(error);
                (DesiredConfig::empty(), Lifecycle::Degraded)
            }
        };

        let lifecycle = if store_ready.is_err() {
            Lifecycle::Degraded
        } else {
            lifecycle
        };
        if let Err(error) = &store_ready {
            error!(%error, "failed to initialize persistent state directory");
        }

        let app = Arc::new(Self {
            backend,
            store,
            inner: Mutex::new(Inner {
                config,
                lifecycle,
                maintenance: false,
                maintenance_exiting: false,
                maintenance_reason: None,
                observed: BTreeMap::new(),
                runtime_healthy: false,
                active_operation: None,
                last_user_operation: last,
                last_system_error: startup_error,
                event_seq: 0,
                boot_id,
                health: HealthState::default(),
                repair_failures: 0,
            }),
            event_tx,
            shutdown: AtomicBool::new(false),
            op_counter: AtomicU64::new(1),
            timing,
        });

        app.initialize();
        app
    }

    fn initialize(self: &Arc<Self>) {
        let lifecycle = self.inner.lock().expect("app state poisoned").lifecycle;
        if matches!(lifecycle, Lifecycle::NeedsSetup | Lifecycle::Degraded) {
            self.refresh_observed();
            self.publish();
            return;
        }

        match self.ensure_session() {
            Ok(_) => {
                let mut inner = self.inner.lock().expect("app state poisoned");
                inner.health.ubus = "ok".into();
                inner.health.rpcd = "ok".into();
            }
            Err(error) => {
                error!(%error, "failed to establish rpcd transaction session");
                let mut inner = self.inner.lock().expect("app state poisoned");
                inner.lifecycle = Lifecycle::Degraded;
                inner.health.ubus = "error".into();
                inner.health.rpcd = "error".into();
                inner.last_system_error = Some(error.clone());
                drop(inner);
                self.publish();
                return;
            }
        }

        match self.store.load_transaction() {
            Ok(Some(journal)) => {
                if let Err(error) = self.recover_from_journal(&journal) {
                    error!(%error, "transaction recovery failed");
                    self.mark_degraded(error);
                    return;
                }
            }
            Ok(None) => {}
            Err(error) => {
                error!(%error, "failed to load transaction journal");
                self.mark_degraded(error);
                return;
            }
        }

        self.refresh_observed();
        let should_repair = {
            let inner = self.inner.lock().expect("app state poisoned");
            !inner.config.wifi.primary.targets.is_empty()
                && !all_equal(
                    &inner.observed,
                    &inner.config.wifi.primary.targets,
                    &inner.config.wifi.primary.ssid,
                )
        };

        if should_repair {
            if let Err(error) = transaction::run_recovery_sync(self, OperationSource::Recovery) {
                error!(%error, "initial reconciliation failed");
                self.mark_degraded(error);
                return;
            }
            self.refresh_observed();
        }

        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if inner.lifecycle == Lifecycle::Booting {
                inner.lifecycle = Lifecycle::Ready;
            }
        }
        self.publish();
    }

    fn recover_from_journal(&self, journal: &TransactionJournal) -> Result<(), DomainError> {
        let config = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .clone();

        if config.revision == journal.base_revision && config.wifi.primary.ssid == journal.old_ssid
        {
            info!(
                operation_id = %journal.operation_id,
                "recovering interrupted transaction to old desired state"
            );
            transaction::force_state_sync(
                self,
                &journal.targets,
                &journal.old_ssid,
                OperationSource::Recovery,
            )?;
            self.record_recovered_operation(
                journal,
                OperationStatus::Failed,
                journal.base_revision,
                Some(
                    DomainError::new(
                        ErrorCode::OperationInterrupted,
                        ErrorStage::Bootstrap,
                        "operation was interrupted by a core restart and rolled back",
                    )
                    .with_operation(&journal.operation_id, Some(&journal.request_id)),
                ),
            )?;
            self.store.clear_transaction()?;
            return Ok(());
        }

        if config.revision == journal.target_revision
            && config.wifi.primary.ssid == journal.new_ssid
        {
            info!(
                operation_id = %journal.operation_id,
                "recovering committed intent to new desired state"
            );
            transaction::force_state_sync(
                self,
                &journal.targets,
                &journal.new_ssid,
                OperationSource::Recovery,
            )?;
            self.record_recovered_operation(
                journal,
                OperationStatus::Succeeded,
                journal.target_revision,
                None,
            )?;
            self.store.clear_transaction()?;
            return Ok(());
        }

        Err(DomainError::new(
            ErrorCode::StateCorrupt,
            ErrorStage::Bootstrap,
            "transaction journal does not match durable desired-state revision",
        )
        .details(json!({
            "config_revision": config.revision,
            "base_revision": journal.base_revision,
            "target_revision": journal.target_revision
        })))
    }

    fn record_recovered_operation(
        &self,
        journal: &TransactionJournal,
        status: OperationStatus,
        revision: u64,
        error: Option<DomainError>,
    ) -> Result<(), DomainError> {
        if journal.source != OperationSource::User {
            return Ok(());
        }

        let last = LastOperation {
            id: journal.operation_id.clone(),
            request_id: Some(journal.request_id.clone()),
            source: OperationSource::User,
            kind: "wifi.set_ssid".into(),
            status,
            revision,
            requested_ssid: journal.new_ssid.clone(),
            error,
            finished_at_ms: now_ms(),
        };
        self.store.persist_last_operation(&last)?;
        self.inner
            .lock()
            .expect("app state poisoned")
            .last_user_operation = Some(last);
        Ok(())
    }

    pub fn start_background(self: &Arc<Self>) {
        let app = Arc::clone(self);
        thread::Builder::new()
            .name("unetic-reconciler".into())
            .spawn(move || app.reconciler_loop())
            .expect("failed to start reconciler");
    }

    fn reconciler_loop(self: Arc<Self>) {
        while !self.shutdown.load(Ordering::Relaxed) {
            thread::sleep(self.timing.reconcile_interval);
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }
            self.reconcile_once();
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    pub fn state(&self) -> PublicState {
        let inner = self.inner.lock().expect("app state poisoned");
        snapshot(&inner)
    }

    pub fn wifi_get(&self) -> WifiPublicState {
        self.state().wifi
    }

    pub fn last_or_active_operation(&self) -> serde_json::Value {
        let state = self.state();
        json!({
            "active": state.active_operation,
            "last": state.last_user_operation
        })
    }

    pub fn health(&self) -> HealthState {
        self.state().health
    }

    pub fn maintenance_get(&self) -> MaintenanceState {
        self.state().maintenance
    }

    pub fn maintenance_enter(&self, reason: Option<String>) -> Result<PublicState, DomainError> {
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if inner.active_operation.is_some() || inner.maintenance_exiting {
                return Err(DomainError::new(
                    ErrorCode::Busy,
                    ErrorStage::Validate,
                    "cannot enter maintenance while another transition is active",
                ));
            }
            if matches!(inner.lifecycle, Lifecycle::Booting | Lifecycle::NeedsSetup) {
                return Err(DomainError::new(
                    ErrorCode::NotReady,
                    ErrorStage::Validate,
                    "maintenance is unavailable before Unetic has initialized",
                ));
            }
            inner.maintenance = true;
            inner.maintenance_exiting = false;
            inner.maintenance_reason = reason;
            inner.lifecycle = Lifecycle::Maintenance;
        }
        Ok(self.publish())
    }

    pub fn maintenance_exit(self: &Arc<Self>) -> Result<PublicState, DomainError> {
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if inner.active_operation.is_some() {
                return Err(DomainError::new(
                    ErrorCode::Busy,
                    ErrorStage::Validate,
                    "cannot exit maintenance while an operation is active",
                ));
            }
            if !inner.maintenance {
                return Ok(snapshot(&inner));
            }
            if inner.maintenance_exiting {
                return Ok(snapshot(&inner));
            }
            inner.maintenance_exiting = true;
        }

        let state = self.publish();
        let app = Arc::clone(self);
        if let Err(spawn_error) = thread::Builder::new()
            .name("unetic-maintenance-exit".into())
            .spawn(move || app.finish_maintenance_exit())
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.maintenance_exiting = false;
            drop(inner);
            self.publish();
            return Err(DomainError::new(
                ErrorCode::Internal,
                ErrorStage::Internal,
                format!("failed to start maintenance reconciliation: {spawn_error}"),
            ));
        }
        Ok(state)
    }

    fn finish_maintenance_exit(&self) {
        // Keep maintenance enabled while the authoritative desired state is restored.
        // This prevents a user mutation from racing the exit reconciliation.
        if let Err(error) = transaction::run_recovery_sync(self, OperationSource::Reconcile) {
            error!(%error, "failed to restore desired state while leaving maintenance");
            {
                let mut inner = self.inner.lock().expect("app state poisoned");
                inner.maintenance = false;
                inner.maintenance_exiting = false;
                inner.maintenance_reason = None;
                inner.lifecycle = Lifecycle::Degraded;
                inner.health.core = "error".into();
                inner.last_system_error = Some(
                    DomainError::new(
                        ErrorCode::ReconcileFailed,
                        ErrorStage::Reconcile,
                        format!("failed to leave maintenance: {}", error.message),
                    )
                    .retryable(true),
                );
                inner.repair_failures = inner.repair_failures.saturating_add(1);
            }
            self.refresh_observed();
            self.publish();
            return;
        }

        self.refresh_observed();
        let config = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .clone();
        if let Err(error) = self.store.persist_config(&config) {
            error!(%error, "state store validation failed while leaving maintenance");
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.maintenance = false;
            inner.maintenance_exiting = false;
            inner.maintenance_reason = None;
            inner.lifecycle = Lifecycle::Degraded;
            inner.health.core = "error".into();
            inner.last_system_error = Some(error.clone());
            drop(inner);
            self.publish();
            return;
        }

        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.maintenance = false;
            inner.maintenance_exiting = false;
            inner.maintenance_reason = None;
            inner.lifecycle = Lifecycle::Ready;
            inner.repair_failures = 0;
            inner.health.core = "ok".into();
            inner.last_system_error = None;
        }
        self.publish();
    }

    pub fn set_ssid(
        self: &Arc<Self>,
        request: SetSsidRequest,
    ) -> Result<OperationAccepted, DomainError> {
        validate_ssid(&request.ssid)?;
        if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
            return Err(DomainError::new(
                ErrorCode::InvalidArgument,
                ErrorStage::Validate,
                "request_id must be between 1 and 128 bytes",
            ));
        }

        let context = {
            let inner = self.inner.lock().expect("app state poisoned");

            if inner.maintenance {
                return Err(DomainError::new(
                    ErrorCode::MaintenanceMode,
                    ErrorStage::Validate,
                    "Unetic is in maintenance mode",
                ));
            }
            if inner.lifecycle != Lifecycle::Ready {
                return Err(DomainError::new(
                    ErrorCode::NotReady,
                    ErrorStage::Validate,
                    format!("core is not ready: {:?}", inner.lifecycle),
                ));
            }

            if let Some(active) = &inner.active_operation {
                if active.request_id.as_deref() == Some(&request.request_id) {
                    if active.requested_ssid != request.ssid {
                        return Err(DomainError::new(
                            ErrorCode::IdempotencyConflict,
                            ErrorStage::Validate,
                            "request_id was already used for a different SSID",
                        )
                        .details(json!({
                            "previous_ssid": active.requested_ssid.clone(),
                            "requested_ssid": request.ssid.clone()
                        })));
                    }
                    return Ok(OperationAccepted {
                        operation_id: active.id.clone(),
                        status: active.status,
                        noop: false,
                    });
                }
                return Err(DomainError::new(
                    ErrorCode::Busy,
                    ErrorStage::Validate,
                    "another configuration operation is active",
                ));
            }

            if let Some(last) = &inner.last_user_operation
                && last.request_id.as_deref() == Some(&request.request_id)
            {
                if last.requested_ssid != request.ssid {
                    return Err(DomainError::new(
                        ErrorCode::IdempotencyConflict,
                        ErrorStage::Validate,
                        "request_id was already used for a different SSID",
                    )
                    .details(json!({
                        "previous_ssid": last.requested_ssid.clone(),
                        "requested_ssid": request.ssid.clone()
                    })));
                }
                return Ok(OperationAccepted {
                    operation_id: last.id.clone(),
                    status: last.status,
                    noop: false,
                });
            }

            if request.expected_revision != inner.config.revision {
                return Err(DomainError::new(
                    ErrorCode::RevisionConflict,
                    ErrorStage::Validate,
                    "configuration changed since this client last synchronized",
                )
                .details(json!({
                    "expected_revision": request.expected_revision,
                    "current_revision": inner.config.revision
                })));
            }

            if request.ssid == inner.config.wifi.primary.ssid {
                return Ok(OperationAccepted {
                    operation_id: self.next_operation_id(),
                    status: OperationStatus::Succeeded,
                    noop: true,
                });
            }

            if inner.config.wifi.primary.targets.is_empty() {
                return Err(DomainError::new(
                    ErrorCode::TargetMissing,
                    ErrorStage::Validate,
                    "primary Wi-Fi has no managed targets",
                ));
            }

            ChangeContext {
                operation_id: self.next_operation_id(),
                request_id: Some(request.request_id.clone()),
                source: OperationSource::User,
                base_revision: inner.config.revision,
                target_revision: inner.config.revision + 1,
                old_ssid: inner.config.wifi.primary.ssid.clone(),
                new_ssid: request.ssid.clone(),
                targets: inner.config.wifi.primary.targets.clone(),
            }
        };

        let journal = context.to_journal(OperationStatus::Accepted);
        self.store.persist_transaction(&journal).map_err(|error| {
            error.with_operation(&context.operation_id, context.request_id.as_deref())
        })?;

        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
        }
        self.publish();

        let operation_id = context.operation_id.clone();
        let app = Arc::clone(self);
        let worker_context = context.clone();
        if let Err(spawn_error) = thread::Builder::new()
            .name(format!(
                "unetic-op-{}",
                &operation_id[..operation_id.len().min(24)]
            ))
            .spawn(move || transaction::run_change(app, worker_context))
        {
            let error = DomainError::new(
                ErrorCode::Internal,
                ErrorStage::Internal,
                format!("failed to start transaction worker: {spawn_error}"),
            )
            .with_operation(&context.operation_id, context.request_id.as_deref());
            self.complete_failure(&context, error.clone(), false);
            return Err(error);
        }

        Ok(OperationAccepted {
            operation_id,
            status: OperationStatus::Accepted,
            noop: false,
        })
    }

    pub(crate) fn ensure_session(&self) -> Result<String, DomainError> {
        // A fresh short-lived rpcd session per transaction avoids stale session
        // state after rpcd restarts and keeps UCI staging isolated by operation.
        self.backend.create_session()
    }

    pub(crate) fn set_operation_status(
        &self,
        context: &ChangeContext,
        status: OperationStatus,
        error: Option<DomainError>,
        persist_journal: bool,
    ) -> Result<(), DomainError> {
        // The accepted journal is the durability boundary. Later phase updates
        // improve diagnostics but are not required for recovery, which is based
        // on base/target revisions and desired state.
        if persist_journal
            && context.source == OperationSource::User
            && let Err(store_error) = self.store.persist_transaction(&context.to_journal(status))
        {
            warn!(
                %store_error,
                operation_id = %context.operation_id,
                "failed to persist transaction phase; continuing with accepted journal"
            );
        }
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if let Some(active) = &mut inner.active_operation
                && active.id == context.operation_id
            {
                active.status = status;
                active.error = error;
            }
        }
        self.publish();
        Ok(())
    }

    pub(crate) fn persist_new_desired(&self, context: &ChangeContext) -> Result<(), DomainError> {
        let mut config = {
            let inner = self.inner.lock().expect("app state poisoned");
            inner.config.clone()
        };
        config.revision = context.target_revision;
        config.wifi.primary.ssid.clone_from(&context.new_ssid);
        self.store.persist_config(&config)?;
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.config = config;
        }
        Ok(())
    }

    pub(crate) fn complete_success(&self, context: &ChangeContext) -> Result<(), DomainError> {
        let revision = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .revision;
        let last = LastOperation {
            id: context.operation_id.clone(),
            request_id: context.request_id.clone(),
            source: context.source,
            kind: "wifi.set_ssid".into(),
            status: OperationStatus::Succeeded,
            revision,
            requested_ssid: context.new_ssid.clone(),
            error: None,
            finished_at_ms: now_ms(),
        };

        let mut completion_store_error = None;
        if context.source == OperationSource::User {
            if let Err(error) = self.store.persist_last_operation(&last) {
                error!(%error, "configuration was committed but last-operation persistence failed");
                completion_store_error = Some(error);
            }
            if let Err(error) = self.store.clear_transaction() {
                error!(%error, "configuration was committed but transaction journal cleanup failed");
                completion_store_error.get_or_insert(error);
            }
        }

        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if context.source == OperationSource::User {
                inner.last_user_operation = Some(last);
            }
            inner.active_operation = None;
            inner.observed = context
                .targets
                .iter()
                .map(|target| (target.clone(), context.new_ssid.clone()))
                .collect();
            inner.runtime_healthy = true;
            inner.repair_failures = 0;
            if let Some(store_error) = completion_store_error {
                inner.lifecycle = Lifecycle::Degraded;
                inner.health.core = "error".into();
                inner.last_system_error = Some(store_error);
            } else {
                if !inner.maintenance {
                    inner.lifecycle = Lifecycle::Ready;
                }
                inner.health.core = "ok".into();
                if context.source != OperationSource::User {
                    inner.last_system_error = None;
                }
            }
            inner.health.wireless = "ok".into();
        }
        self.publish();

        // The configuration is already confirmed at this point. Do not turn a
        // bookkeeping failure into a fake rollback/failure of the user's change.
        Ok(())
    }

    pub(crate) fn complete_failure(
        &self,
        context: &ChangeContext,
        error: DomainError,
        rollback_failed: bool,
    ) {
        let error = error.with_operation(&context.operation_id, context.request_id.as_deref());
        let revision = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .revision;
        let last = LastOperation {
            id: context.operation_id.clone(),
            request_id: context.request_id.clone(),
            source: context.source,
            kind: "wifi.set_ssid".into(),
            status: if rollback_failed {
                OperationStatus::RollbackFailed
            } else {
                OperationStatus::Failed
            },
            revision,
            requested_ssid: context.new_ssid.clone(),
            error: Some(error.clone()),
            finished_at_ms: now_ms(),
        };

        let mut store_failed = false;
        if context.source == OperationSource::User {
            if let Err(store_error) = self.store.persist_last_operation(&last) {
                error!(%store_error, "failed to persist last operation");
                store_failed = true;
            }
            if !rollback_failed && let Err(store_error) = self.store.clear_transaction() {
                error!(%store_error, "failed to clear transaction journal");
                store_failed = true;
            }
        }

        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if context.source == OperationSource::User {
                inner.last_user_operation = Some(last);
                if store_failed {
                    inner.last_system_error = Some(DomainError::new(
                        ErrorCode::StateStoreFailed,
                        ErrorStage::Persist,
                        "failed to persist operation result",
                    ));
                }
            } else {
                inner.repair_failures = inner.repair_failures.saturating_add(1);
                inner.last_system_error = Some(error.clone());
            }
            inner.active_operation = None;
            if rollback_failed || store_failed || inner.repair_failures >= 3 {
                inner.lifecycle = Lifecycle::Degraded;
                inner.health.core = "error".into();
            }
            inner.health.wireless = "error".into();
        }
        self.refresh_observed();
        self.publish();
    }

    pub(crate) fn mark_commit_uncertain(&self, context: &ChangeContext, error: DomainError) {
        let uncertain = DomainError::new(
            ErrorCode::CommitUncertain,
            ErrorStage::Confirm,
            format!(
                "desired state is durable, but UCI confirm failed: {}",
                error.message
            ),
        )
        .retryable(true)
        .with_operation(&context.operation_id, context.request_id.as_deref());

        let revision = self
            .inner
            .lock()
            .expect("app state poisoned")
            .config
            .revision;
        let last = LastOperation {
            id: context.operation_id.clone(),
            request_id: context.request_id.clone(),
            source: context.source,
            kind: "wifi.set_ssid".into(),
            status: OperationStatus::Failed,
            revision,
            requested_ssid: context.new_ssid.clone(),
            error: Some(uncertain.clone()),
            finished_at_ms: now_ms(),
        };

        if context.source == OperationSource::User {
            let _ = self.store.persist_last_operation(&last);
        }
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            if context.source == OperationSource::User {
                inner.last_user_operation = Some(last);
            }
            inner.active_operation = None;
            inner.lifecycle = Lifecycle::Degraded;
            inner.health.core = "error".into();
            inner.last_system_error = Some(uncertain.clone());
        }
        self.publish();
    }

    pub(crate) fn mark_degraded(&self, error: DomainError) {
        error!(%error, "core entered degraded mode");
        {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.lifecycle = Lifecycle::Degraded;
            inner.health.core = "error".into();
            inner.last_system_error = Some(error);
        }
        self.publish();
    }

    pub(crate) fn refresh_observed(&self) -> bool {
        let (targets, desired) = {
            let inner = self.inner.lock().expect("app state poisoned");
            (
                inner.config.wifi.primary.targets.clone(),
                inner.config.wifi.primary.ssid.clone(),
            )
        };

        if targets.is_empty() {
            return false;
        }

        match self.backend.read_ssids(&targets, None) {
            Ok(observed) => {
                let (runtime, runtime_error) =
                    match self.backend.runtime_healthy(&targets, &desired) {
                        Ok(value) => (value, None),
                        Err(error) => {
                            warn!(%error, "failed to observe wireless runtime");
                            (false, Some(error))
                        }
                    };
                let mut inner = self.inner.lock().expect("app state poisoned");
                let wireless_health = if runtime { "ok" } else { "error" };
                let changed = inner.observed != observed
                    || inner.runtime_healthy != runtime
                    || inner.health.wireless != wireless_health
                    || runtime_error
                        .as_ref()
                        .is_some_and(|error| inner.last_system_error.as_ref() != Some(error));
                inner.observed = observed;
                inner.runtime_healthy = runtime;
                inner.health.wireless = wireless_health.into();
                if let Some(error) = runtime_error {
                    inner.last_system_error = Some(error);
                }
                changed
            }
            Err(error) => {
                warn!(%error, "failed to observe wireless config");
                let mut inner = self.inner.lock().expect("app state poisoned");
                let changed = inner.health.wireless != "error"
                    || inner.last_system_error.as_ref() != Some(&error);
                inner.health.wireless = "error".into();
                inner.last_system_error = Some(error);
                changed
            }
        }
    }

    fn reconcile_once(self: &Arc<Self>) {
        let observation_changed = self.refresh_observed();

        enum Repair {
            Config(ChangeContext),
            Runtime { targets: Vec<String>, ssid: String },
            None { publish: bool },
        }

        let repair = {
            let mut inner = self.inner.lock().expect("app state poisoned");
            let config_drift = !all_equal(
                &inner.observed,
                &inner.config.wifi.primary.targets,
                &inner.config.wifi.primary.ssid,
            );
            let runtime_drift = !inner.runtime_healthy;

            if inner.maintenance {
                drop(inner);
                if observation_changed {
                    self.publish();
                }
                return;
            }

            if inner.lifecycle == Lifecycle::Degraded && inner.repair_failures >= 3 {
                drop(inner);
                if observation_changed {
                    self.publish();
                }
                return;
            }

            if inner.active_operation.is_some()
                || inner.config.wifi.primary.targets.is_empty()
                || matches!(inner.lifecycle, Lifecycle::NeedsSetup | Lifecycle::Booting)
            {
                drop(inner);
                if observation_changed {
                    self.publish();
                }
                return;
            }

            if !config_drift && !runtime_drift {
                let mut publish = observation_changed;
                if inner.repair_failures > 0 {
                    inner.repair_failures = 0;
                    if inner.lifecycle == Lifecycle::Degraded {
                        inner.lifecycle = Lifecycle::Ready;
                    }
                    inner.health.core = "ok".into();
                    inner.last_system_error = None;
                    publish = true;
                } else if inner.lifecycle == Lifecycle::Ready && inner.last_system_error.is_some() {
                    inner.last_system_error = None;
                    publish = true;
                }
                Repair::None { publish }
            } else if config_drift {
                let context = ChangeContext {
                    operation_id: self.next_operation_id(),
                    request_id: None,
                    source: OperationSource::Reconcile,
                    base_revision: inner.config.revision,
                    target_revision: inner.config.revision,
                    old_ssid: inner.config.wifi.primary.ssid.clone(),
                    new_ssid: inner.config.wifi.primary.ssid.clone(),
                    targets: inner.config.wifi.primary.targets.clone(),
                };
                inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
                Repair::Config(context)
            } else {
                Repair::Runtime {
                    targets: inner.config.wifi.primary.targets.clone(),
                    ssid: inner.config.wifi.primary.ssid.clone(),
                }
            }
        };

        match repair {
            Repair::Config(context) => {
                self.publish();
                let app = Arc::clone(self);
                if thread::Builder::new()
                    .name("unetic-reconcile".into())
                    .spawn(move || transaction::run_change(app, context))
                    .is_err()
                {
                    let mut inner = self.inner.lock().expect("app state poisoned");
                    inner.active_operation = None;
                    inner.repair_failures = inner.repair_failures.saturating_add(1);
                    let error = DomainError::new(
                        ErrorCode::ReconcileFailed,
                        ErrorStage::Reconcile,
                        "failed to start reconciliation worker",
                    )
                    .retryable(true);
                    inner.last_system_error = Some(error);
                    if inner.repair_failures >= 3 {
                        inner.lifecycle = Lifecycle::Degraded;
                        inner.health.core = "error".into();
                    }
                    drop(inner);
                    self.publish();
                }
            }
            Repair::Runtime { targets, ssid } => {
                if let Err(error) = self.repair_runtime(&targets, &ssid) {
                    warn!(%error, "runtime-only wireless repair failed");
                    let mut inner = self.inner.lock().expect("app state poisoned");
                    inner.repair_failures = inner.repair_failures.saturating_add(1);
                    inner.last_system_error = Some(error.clone());
                    if inner.repair_failures >= 3 {
                        inner.lifecycle = Lifecycle::Degraded;
                        inner.health.core = "error".into();
                    }
                    inner.health.wireless = "error".into();
                    drop(inner);
                    self.publish();
                } else {
                    let mut inner = self.inner.lock().expect("app state poisoned");
                    inner.repair_failures = 0;
                    if inner.lifecycle == Lifecycle::Degraded {
                        inner.lifecycle = Lifecycle::Ready;
                    }
                    inner.health.core = "ok".into();
                    inner.health.wireless = "ok".into();
                    inner.runtime_healthy = true;
                    inner.last_system_error = None;
                    drop(inner);
                    self.publish();
                }
            }
            Repair::None { publish } => {
                if publish {
                    self.publish();
                }
            }
        }
    }

    fn repair_runtime(&self, targets: &[String], ssid: &str) -> Result<(), DomainError> {
        self.backend.reload_wireless_runtime()?;
        let deadline = std::time::Instant::now() + self.timing.rollback_verify_timeout;
        let mut successful_samples = 0_u8;
        while std::time::Instant::now() < deadline {
            match self.backend.runtime_healthy(targets, ssid) {
                Ok(true) => {
                    successful_samples = successful_samples.saturating_add(1);
                    if successful_samples >= 2 {
                        self.refresh_observed();
                        return Ok(());
                    }
                }
                Ok(false) | Err(_) => successful_samples = 0,
            }
            thread::sleep(self.timing.verify_sample_delay);
        }
        Err(DomainError::new(
            ErrorCode::ReconcileFailed,
            ErrorStage::Reconcile,
            "wireless runtime did not recover after reload",
        )
        .retryable(true))
    }

    pub(crate) fn publish(&self) -> PublicState {
        let state = {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.event_seq = inner.event_seq.saturating_add(1);
            snapshot(&inner)
        };
        let _ = self.event_tx.send(state.clone());
        state
    }

    fn next_operation_id(&self) -> String {
        let count = self.op_counter.fetch_add(1, Ordering::Relaxed);
        format!("op-{}-{count}", now_ms())
    }
}

fn snapshot(inner: &Inner) -> PublicState {
    let desired = &inner.config.wifi.primary;
    let mut drift_fields: Vec<String> = desired
        .targets
        .iter()
        .filter(|target| inner.observed.get(*target) != Some(&desired.ssid))
        .map(|target| format!("wifi.primary.targets.{target}.ssid"))
        .collect();
    if !inner.runtime_healthy && !desired.targets.is_empty() {
        drift_fields.push("wifi.primary.runtime".into());
    }
    let drifted = !drift_fields.is_empty();

    let wifi_status = if inner.active_operation.is_some() {
        WifiStatus::Applying
    } else if drifted {
        WifiStatus::Drifted
    } else if inner.observed.is_empty() {
        WifiStatus::Unknown
    } else {
        WifiStatus::Synced
    };

    PublicState {
        api_version: API_VERSION,
        core_version: env!("CARGO_PKG_VERSION").into(),
        boot_id: inner.boot_id.clone(),
        event_seq: inner.event_seq,
        revision: inner.config.revision,
        lifecycle: inner.lifecycle,
        maintenance: MaintenanceState {
            enabled: inner.maintenance,
            exiting: inner.maintenance_exiting,
            reason: inner.maintenance_reason.clone(),
        },
        wifi: WifiPublicState {
            ssid: desired.ssid.clone(),
            targets: desired.targets.clone(),
            observed: inner.observed.clone(),
            status: wifi_status,
        },
        active_operation: inner.active_operation.clone(),
        last_user_operation: inner.last_user_operation.clone(),
        last_system_error: inner.last_system_error.clone(),
        drift: DriftState {
            detected: drifted,
            fields: drift_fields,
        },
        health: inner.health.clone(),
    }
}

fn all_equal(observed: &BTreeMap<String, String>, targets: &[String], expected: &str) -> bool {
    !targets.is_empty()
        && targets
            .iter()
            .all(|target| observed.get(target).is_some_and(|ssid| ssid == expected))
}

fn validate_ssid(ssid: &str) -> Result<(), DomainError> {
    if ssid.is_empty() {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "SSID must not be empty",
        ));
    }
    if ssid.len() > 32 {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "SSID must be at most 32 UTF-8 bytes",
        )
        .details(json!({"bytes": ssid.len()})));
    }
    if ssid.contains('\0') {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "SSID must not contain NUL",
        ));
    }
    Ok(())
}

fn generate_id(prefix: &str) -> String {
    if let Ok(value) = fs::read_to_string("/proc/sys/kernel/random/uuid") {
        return format!("{prefix}-{}", value.trim());
    }
    format!("{prefix}-{}-{}", std::process::id(), now_ms())
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().try_into().unwrap_or(u64::MAX)
        })
}
