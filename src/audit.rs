//! An audit trail, as a [`Hook`].
//!
//! A REST service that does not record its authorization decisions is a
//! liability: when someone asks who deleted a row, "the logs" has to be an
//! answer rather than a shrug. This module makes that answer a two-line
//! registration.
//!
//! It is deliberately *not* a second interception mechanism. An audit record
//! is exactly an [`after`](crate::hook::Hook::after) hook — it observes an
//! operation and its outcome, and cannot change either — so it is built on
//! the same [`Hooks`](crate::hook::Hooks) every other cross-cutting concern
//! uses. Metrics and tracing will be the same shape.
//!
//! ```ignore
//! use ash_jsonapi::audit::Audit;
//! use ash_jsonapi::crud::Validators;
//! use ash_jsonapi::hook::Hooks;
//!
//! let hooks = Hooks::new().with(Audit::new(logger));
//!
//! impl Validators for App {
//!     fn hooks(&self) -> Option<&Hooks> {
//!         Some(&self.hooks)
//!     }
//! }
//! ```
//!
//! # What is recorded
//!
//! One event per operation, carrying the correlation id, the principal, the
//! resource and operation, and the outcome. The record is written *after*
//! the operation, so it states what actually happened rather than what was
//! attempted.
//!
//! Attribute values are never recorded. An audit trail says that
//! `alice@example.com` updated `todo/t_1` and that it succeeded; what she
//! changed it to is the row's business, and a log that copies request bodies
//! is a second place for personal data to leak from.

use std::sync::Arc;

use ash_log::{AuditEvent, AuditEventType, AuditResult, AuditSeverity, Logger};

use crate::context::Context;
use crate::hook::{Event, Hook, Outcome};

/// Records every operation to an `ash-log` [`Logger`].
///
/// Register it on your [`Hooks`](crate::hook::Hooks); see the module docs.
pub struct Audit {
    logger: Arc<Logger>,
}

impl Audit {
    /// Record to `logger`.
    pub fn new(logger: Arc<Logger>) -> Self {
        Self { logger }
    }
}

impl Hook for Audit {
    fn after(&self, event: &Event<'_>, outcome: &Outcome, ctx: &Context) {
        // `method` is the operation as a client would name it — `read todo`,
        // `delete todo/t_1` — so a record is legible without knowing this
        // crate's internals.
        let method = match event.id {
            Some(id) => format!("{} {}/{}", event.operation.as_str(), event.resource, id),
            None => format!("{} {}", event.operation.as_str(), event.resource),
        };

        let mut builder = AuditEvent::builder()
            .event_type(event_type(outcome))
            .result(result(outcome))
            .severity(severity(event, outcome))
            .correlation_id(ctx.correlation_id())
            .method(method);

        // Absent until authentication has run: an unauthenticated request is
        // recorded as one rather than as an empty principal.
        if let Some(principal) = ctx.principal() {
            builder = builder.principal(principal);
        }

        self.logger.log(builder.build());
    }
}

/// How to classify the record.
///
/// A `403` is an authorization decision, which is the event type a compliance
/// review actually searches for; everything else is an invocation or an
/// error.
fn event_type(outcome: &Outcome) -> AuditEventType {
    match outcome {
        Outcome::Succeeded { .. } => AuditEventType::MethodInvocation,
        Outcome::Failed { status: 401 | 403 } => AuditEventType::AuthorizationCheck,
        Outcome::Failed { .. } => AuditEventType::ErrorOccurred,
    }
}

/// Denied, failed, or succeeded — the three a reviewer cares to distinguish.
fn result(outcome: &Outcome) -> AuditResult {
    match outcome {
        Outcome::Succeeded { .. } => AuditResult::Success,
        Outcome::Failed { status: 401 | 403 } => AuditResult::Denied,
        Outcome::Failed { .. } => AuditResult::Failure,
    }
}

/// How loud the record is.
///
/// A denied *write* is the one worth waking someone for: a denied read is
/// often just a client probing, but a denied delete is someone trying to
/// remove data they do not own. A server error is likewise a warning, since
/// it is the service's own fault.
fn severity(event: &Event<'_>, outcome: &Outcome) -> AuditSeverity {
    match outcome {
        Outcome::Succeeded { .. } => AuditSeverity::Info,
        Outcome::Failed { status: 401 | 403 } if event.operation.is_write() => {
            AuditSeverity::Warning
        }
        Outcome::Failed { status } if *status >= 500 => AuditSeverity::Warning,
        Outcome::Failed { .. } => AuditSeverity::Info,
    }
}
