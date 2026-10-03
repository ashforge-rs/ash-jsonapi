use ash_jsonapi::hook::{Event, Hook, Hooks, Operation, Outcome};
use ash_jsonapi::{Context, ErrorCode, JsonApiError};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Hook for Recorder {
    fn before(&self, e: &Event<'_>, _c: &Context) -> Result<(), JsonApiError> {
        self.0
            .lock()
            .unwrap()
            .push(format!("before:{}", e.operation.as_str()));
        Ok(())
    }
    fn after(&self, e: &Event<'_>, o: &Outcome, _c: &Context) {
        self.0
            .lock()
            .unwrap()
            .push(format!("after:{}:{}", e.operation.as_str(), o.status()));
    }
}

struct Deny;
impl Hook for Deny {
    fn before(&self, _e: &Event<'_>, ctx: &Context) -> Result<(), JsonApiError> {
        Err(JsonApiError::in_request(ErrorCode::Forbidden, ctx))
    }
}

fn ev() -> Event<'static> {
    Event {
        operation: Operation::Create,
        resource: "todo",
        id: None,
    }
}

#[test]
fn multiple_hooks_run_in_registration_order() {
    let r = Recorder::default();
    let hooks = Hooks::new().with(r.clone()).with(r.clone());
    let ctx = Context::new("c");

    assert!(hooks.before(&ev(), &ctx).is_ok());
    hooks.after(&ev(), &Outcome::Succeeded { status: 200 }, &ctx);

    let log = r.0.lock().unwrap().clone();
    assert_eq!(
        log,
        vec![
            "before:create",
            "before:create",
            "after:create:200",
            "after:create:200"
        ]
    );
}

#[test]
fn a_before_hook_can_refuse() {
    let ctx = Context::new("c");
    let hooks = Hooks::new().with(Deny);
    let err = hooks.before(&ev(), &ctx).unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
}

#[test]
fn refusal_stops_later_before_hooks() {
    let r = Recorder::default();
    // Deny is registered first, so the recorder must never see `before`.
    let hooks = Hooks::new().with(Deny).with(r.clone());
    let ctx = Context::new("c");

    assert!(hooks.before(&ev(), &ctx).is_err());
    assert!(r.0.lock().unwrap().is_empty(), "later hooks must not run");
}

#[test]
fn every_after_hook_runs() {
    let r = Recorder::default();
    let hooks = Hooks::new().with(r.clone()).with(r.clone()).with(r.clone());
    let ctx = Context::new("c");
    hooks.after(&ev(), &Outcome::Failed { status: 500 }, &ctx);
    assert_eq!(r.0.lock().unwrap().len(), 3);
}

#[test]
fn empty_is_the_default_and_is_cheap() {
    let hooks = Hooks::new();
    assert!(hooks.is_empty());
    assert!(!Hooks::new().with(Deny).is_empty());
}

#[test]
fn hooks_survive_being_cloned_and_shared() {
    // `with` uses Arc::get_mut; make sure cloning before/after building is safe.
    let hooks = Hooks::new().with(Recorder::default());
    let cloned = hooks.clone();
    let ctx = Context::new("c");
    assert!(cloned.before(&ev(), &ctx).is_ok());
    assert!(hooks.before(&ev(), &ctx).is_ok());
}

#[test]
fn operation_classifies_writes() {
    assert!(Operation::Create.is_write());
    assert!(Operation::Update.is_write());
    assert!(Operation::Delete.is_write());
    assert!(!Operation::List.is_write());
    assert!(!Operation::Read.is_write());
}
