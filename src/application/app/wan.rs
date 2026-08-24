use std::{sync::Arc, thread};

use serde_json::json;

use super::{App, Inner, StateTopic};
use crate::{
    application::state::now_ms,
    application::wan::WanChangeContext,
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{
        LastOperation, Lifecycle, OperationAccepted, OperationIntent, OperationSource,
        OperationStatus, SetWanRequest,
    },
};

impl App {
    pub fn set_wan(
        self: &Arc<Self>,
        mut request: SetWanRequest,
    ) -> Result<OperationAccepted, LegacyAppError> {
        validate_wan_request(&request)?;
        request.wan = crate::application::wan::normalize_wan_desired(request.wan);

        let (context, noop_result) = {
            let mut inner = self.inner.lock().expect("app state poisoned");
            check_wan_app_ready(&inner)?;

            if let Some(accepted) = check_wan_idempotency(&inner, &request)? {
                return Ok(accepted);
            }

            check_wan_revision(inner.config.revision, request.expected_revision)?;

            if inner.config.wan == request.wan {
                let accepted = OperationAccepted {
                    operation_id: self.next_operation_id(),
                    status: OperationStatus::Succeeded,
                    noop: true,
                };
                inner.last_user_operation = Some(noop_last_operation(
                    &accepted,
                    inner.config.revision,
                    &request,
                ));
                (None, Some(accepted))
            } else {
                let context = build_wan_change_context(&inner, self.next_operation_id(), &request);
                inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
                (Some(context), None)
            }
        };

        if let Some(accepted) = noop_result {
            self.publish(StateTopic::Operation);
            return Ok(accepted);
        }

        let context = context.expect("context must be present if not noop");
        self.persist_and_spawn_wan_change(context)
    }

    fn persist_and_spawn_wan_change(
        self: &Arc<Self>,
        context: WanChangeContext,
    ) -> Result<OperationAccepted, LegacyAppError> {
        let journal = context.to_journal(OperationStatus::Accepted);
        if let Err(error) = self.store.persist_transaction(&journal) {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.active_operation = None;
            return Err(error.with_operation(&context.operation_id, context.request_id.as_deref()));
        }
        self.publish(StateTopic::Operation);

        let operation_id = context.operation_id.clone();
        let app = Arc::clone(self);
        let worker_context = context.clone();
        let thread_name = format!("unetic-wan-{}", &operation_id[..operation_id.len().min(24)]);

        if let Err(spawn_error) = thread::Builder::new()
            .name(thread_name)
            .spawn(move || crate::application::wan::run_wan_change(app, worker_context))
        {
            let error = LegacyAppError::new(
                ErrorCode::Internal,
                ErrorStage::Internal,
                format!("failed to start transaction worker: {spawn_error}"),
            )
            .with_operation(&context.operation_id, context.request_id.as_deref());
            self.complete_wan_failure(&context, error.clone(), false);
            return Err(error);
        }

        Ok(OperationAccepted {
            operation_id,
            status: OperationStatus::Accepted,
            noop: false,
        })
    }
}

fn validate_wan_request(request: &SetWanRequest) -> Result<(), LegacyAppError> {
    if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
        return Err(LegacyAppError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "request_id must be between 1 and 128 bytes",
        ));
    }
    crate::application::wan::validate_wan_desired(&request.wan)
}

fn check_wan_app_ready(inner: &Inner) -> Result<(), LegacyAppError> {
    if inner.maintenance {
        return Err(LegacyAppError::new(
            ErrorCode::MaintenanceMode,
            ErrorStage::Validate,
            "Unetic is in maintenance mode",
        ));
    }
    if inner.lifecycle != Lifecycle::Ready {
        return Err(LegacyAppError::new(
            ErrorCode::NotReady,
            ErrorStage::Validate,
            format!("core is not ready: {:?}", inner.lifecycle),
        ));
    }
    Ok(())
}

fn check_wan_idempotency(
    inner: &Inner,
    request: &SetWanRequest,
) -> Result<Option<OperationAccepted>, LegacyAppError> {
    let intent = OperationIntent::Wan(request.wan.clone());
    if let Some(active) = &inner.active_operation {
        if active.request_id.as_deref() == Some(request.request_id.as_str()) {
            if active.intent.as_ref() != Some(&intent) {
                return Err(idempotency_conflict());
            }
            return Ok(Some(OperationAccepted {
                operation_id: active.id.clone(),
                status: active.status,
                noop: false,
            }));
        }
        return Err(LegacyAppError::new(
            ErrorCode::Busy,
            ErrorStage::Validate,
            "another configuration operation is active",
        ));
    }

    if let Some(last) = &inner.last_user_operation
        && last.request_id.as_deref() == Some(request.request_id.as_str())
    {
        if last.intent.as_ref() != Some(&intent) {
            return Err(idempotency_conflict());
        }
        return Ok(Some(OperationAccepted {
            operation_id: last.id.clone(),
            status: last.status,
            noop: false,
        }));
    }

    Ok(None)
}

fn noop_last_operation(
    accepted: &OperationAccepted,
    revision: u64,
    request: &SetWanRequest,
) -> LastOperation {
    LastOperation {
        id: accepted.operation_id.clone(),
        request_id: Some(request.request_id.clone()),
        source: OperationSource::User,
        kind: "wan.set_config".to_owned(),
        status: OperationStatus::Succeeded,
        revision,
        requested_ssid: String::new(),
        intent: Some(OperationIntent::Wan(request.wan.clone())),
        error: None,
        finished_at_ms: now_ms(),
    }
}

fn idempotency_conflict() -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::IdempotencyConflict,
        ErrorStage::Validate,
        "request_id was already used for a different WAN configuration",
    )
}

fn check_wan_revision(current_revision: u64, expected_revision: u64) -> Result<(), LegacyAppError> {
    if expected_revision != current_revision {
        return Err(LegacyAppError::new(
            ErrorCode::RevisionConflict,
            ErrorStage::Validate,
            "configuration changed since this client last synchronized",
        )
        .details(json!({
            "expected_revision": expected_revision,
            "current_revision": current_revision
        })));
    }
    Ok(())
}

fn build_wan_change_context(
    inner: &Inner,
    operation_id: String,
    request: &SetWanRequest,
) -> WanChangeContext {
    WanChangeContext {
        operation_id,
        request_id: Some(request.request_id.clone()),
        source: OperationSource::User,
        base_revision: inner.config.revision,
        target_revision: inner.config.revision + 1,
        old_wan: inner.config.wan.clone(),
        new_wan: request.wan.clone(),
    }
}
