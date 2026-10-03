//! The audit hook must record every operation, classified correctly.
use std::sync::{Arc, Mutex};

use ash_jsonapi::Context;
use ash_jsonapi::audit::Audit;
use ash_jsonapi::hook::{Event, Hook, Operation, Outcome};
use ash_log::{AuditBackend, AuditEvent, AuditEventType, AuditResult, AuditSeverity, Logger};

#[derive(Default)]
struct Capture(Mutex<Vec<AuditEvent>>);
impl AuditBackend for Capture {
    fn log_audit(&self, event: &AuditEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

fn audit_with() -> (Audit, Arc<Capture>) {
    let backend = Arc::new(Capture::default());
    let logger = Logger::builder(backend.clone()).build();
    (Audit::new(Arc::new(logger)), backend)
}

fn ev<'a>(op: Operation, id: Option<&'a str>) -> Event<'a> {
    Event {
        operation: op,
        resource: "todo",
        id,
    }
}

#[test]
fn a_successful_operation_is_recorded() {
    let (audit, cap) = audit_with();
    let ctx = Context::new("cid-1").with_principal("alice@example.com");

    audit.after(
        &ev(Operation::Read, Some("t_1")),
        &Outcome::Succeeded { status: 200 },
        &ctx,
    );

    let events = cap.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.event_type, AuditEventType::MethodInvocation);
    assert_eq!(e.result, AuditResult::Success);
    assert_eq!(e.principal.as_deref(), Some("alice@example.com"));
    assert_eq!(e.correlation_id.as_deref(), Some("cid-1"));
    assert_eq!(e.method.as_deref(), Some("read todo/t_1"));
}

#[test]
fn a_denial_is_an_authorization_check_not_an_error() {
    let (audit, cap) = audit_with();
    let ctx = Context::new("cid-2");
    audit.after(
        &ev(Operation::Read, Some("t_1")),
        &Outcome::Failed { status: 403 },
        &ctx,
    );

    let events = cap.0.lock().unwrap();
    assert_eq!(events[0].event_type, AuditEventType::AuthorizationCheck);
    assert_eq!(events[0].result, AuditResult::Denied);
}

#[test]
fn a_denied_write_is_louder_than_a_denied_read() {
    let (audit, cap) = audit_with();
    let ctx = Context::new("c");
    audit.after(
        &ev(Operation::Read, Some("t")),
        &Outcome::Failed { status: 403 },
        &ctx,
    );
    audit.after(
        &ev(Operation::Delete, Some("t")),
        &Outcome::Failed { status: 403 },
        &ctx,
    );

    let events = cap.0.lock().unwrap();
    assert_eq!(events[0].severity, AuditSeverity::Info, "denied read");
    assert_eq!(events[1].severity, AuditSeverity::Warning, "denied delete");
}

#[test]
fn a_server_error_is_a_warning() {
    let (audit, cap) = audit_with();
    let ctx = Context::new("c");
    audit.after(
        &ev(Operation::List, None),
        &Outcome::Failed { status: 500 },
        &ctx,
    );

    let events = cap.0.lock().unwrap();
    assert_eq!(events[0].event_type, AuditEventType::ErrorOccurred);
    assert_eq!(events[0].result, AuditResult::Failure);
    assert_eq!(events[0].severity, AuditSeverity::Warning);
}

#[test]
fn an_unauthenticated_request_records_no_principal() {
    let (audit, cap) = audit_with();
    let ctx = Context::new("c");
    audit.after(
        &ev(Operation::List, None),
        &Outcome::Succeeded { status: 200 },
        &ctx,
    );

    let events = cap.0.lock().unwrap();
    assert!(events[0].principal.is_none(), "absent, not empty");
    assert_eq!(events[0].method.as_deref(), Some("list todo"));
}
