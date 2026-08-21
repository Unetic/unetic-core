use std::{sync::Arc, thread};

use serde_json::json;

use super::{App, Inner};
use crate::{
    application::wan::WanChangeContext,
    domain::errors::{DomainError, ErrorCode, ErrorStage},
    domain::{Lifecycle, OperationAccepted, OperationSource, OperationStatus, SetWanRequest},
};

impl App {
    pub fn set_wan(
        self: &Arc<Self>,
        request: SetWanRequest,
    ) -> Result<OperationAccepted, DomainError> {
        validate_wan_request(&request)?;

        let (context, noop_result) = {
            let mut inner = self.inner.lock().expect("app state poisoned");
            check_wan_app_ready(&inner)?;

            if let Some(accepted) = check_wan_idempotency(&inner, &request.request_id)? {
                return Ok(accepted);
            }

            check_wan_revision(inner.config.revision, request.expected_revision)?;

            if inner.config.wan == request.wan {
                let accepted = OperationAccepted {
                    operation_id: self.next_operation_id(),
                    status: OperationStatus::Succeeded,
                    noop: true,
                };
                (None, Some(accepted))
            } else {
                let context = build_wan_change_context(&inner, self.next_operation_id(), &request);
                inner.active_operation = Some(context.public(OperationStatus::Accepted, None));
                (Some(context), None)
            }
        };

        if let Some(accepted) = noop_result {
            return Ok(accepted);
        }

        let context = context.expect("context must be present if not noop");
        self.persist_and_spawn_wan_change(context)
    }

    fn persist_and_spawn_wan_change(
        self: &Arc<Self>,
        context: WanChangeContext,
    ) -> Result<OperationAccepted, DomainError> {
        let journal = context.to_journal(OperationStatus::Accepted);
        if let Err(error) = self.store.persist_transaction(&journal) {
            let mut inner = self.inner.lock().expect("app state poisoned");
            inner.active_operation = None;
            return Err(error.with_operation(&context.operation_id, context.request_id.as_deref()));
        }
        self.publish();

        let operation_id = context.operation_id.clone();
        let app = Arc::clone(self);
        let worker_context = context.clone();
        let thread_name = format!("unetic-wan-{}", &operation_id[..operation_id.len().min(24)]);

        if let Err(spawn_error) = thread::Builder::new()
            .name(thread_name)
            .spawn(move || crate::application::wan::run_wan_change(app, worker_context))
        {
            let error = DomainError::new(
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

fn validate_wan_request(request: &SetWanRequest) -> Result<(), DomainError> {
    if request.request_id.trim().is_empty() || request.request_id.len() > 128 {
        return Err(DomainError::new(
            ErrorCode::InvalidArgument,
            ErrorStage::Validate,
            "request_id must be between 1 and 128 bytes",
        ));
    }
    crate::application::wan::validate_wan_desired(&request.wan)
}

fn check_wan_app_ready(inner: &Inner) -> Result<(), DomainError> {
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
    Ok(())
}

fn check_wan_idempotency(
    inner: &Inner,
    request_id: &str,
) -> Result<Option<OperationAccepted>, DomainError> {
    if let Some(active) = &inner.active_operation {
        if active.request_id.as_deref() == Some(request_id) {
            return Ok(Some(OperationAccepted {
                operation_id: active.id.clone(),
                status: active.status,
                noop: false,
            }));
        }
        return Err(DomainError::new(
            ErrorCode::Busy,
            ErrorStage::Validate,
            "another configuration operation is active",
        ));
    }

    if let Some(last) = &inner.last_user_operation
        && last.request_id.as_deref() == Some(request_id)
    {
        return Ok(Some(OperationAccepted {
            operation_id: last.id.clone(),
            status: last.status,
            noop: false,
        }));
    }

    Ok(None)
}

fn check_wan_revision(current_revision: u64, expected_revision: u64) -> Result<(), DomainError> {
    if expected_revision != current_revision {
        return Err(DomainError::new(
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
