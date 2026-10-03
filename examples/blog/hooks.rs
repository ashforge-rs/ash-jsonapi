//! Cross-cutting policy: one place that sees every operation on every
//! resource.
//!
//! A [`Hook`] runs around each operation the generated handlers serve.
//! `before` can refuse one — returning `Err` stops it reaching storage at
//! all — and `after` sees how it ended. Neither knows which resource it is
//! looking at until it is called, which is what makes this the right place
//! for rules that are not about any one resource: authorization, audit, and
//! metrics.
//!
//! The alternative is the same check written at the top of every `create`,
//! `update` and `destroy`, where the one you forget is the one that matters.

use ash_jsonapi::hook::{Event, Hook, Outcome};
use ash_jsonapi::{Context, ErrorCode, JsonApiError};

/// Refuses writes from anyone who is not signed in.
///
/// The check runs in `before`, so a refused write never reaches storage —
/// no row is touched, and no `after` hook records a change that did not
/// happen. Reads are left alone, which is what makes this a policy rather
/// than a lock.
///
/// The principal comes from the [`Context`], which the crate's layer builds
/// per request. Where it comes from *before* that — a session cookie, a
/// bearer token, mTLS — is your service's business; see [`crate::http`],
/// which sets it from a header for the sake of the demo.
pub struct RequireAuthForWrites;

impl Hook for RequireAuthForWrites {
    fn before(&self, event: &Event<'_>, ctx: &Context) -> Result<(), JsonApiError> {
        // `is_write` is true for create, update and delete — so a new write
        // operation is covered the day it is added, rather than the day
        // someone remembers to extend a match here.
        if event.operation.is_write() && ctx.principal().is_none() {
            return Err(JsonApiError::in_request(ErrorCode::Forbidden, ctx)
                .detail("writes require an authenticated principal"));
        }
        Ok(())
    }
}

/// Prints one line per operation, so the demo shows the hook firing.
///
/// This is what [`ash_jsonapi::audit::Audit`] does properly: it is a `Hook`
/// like this one, and its `after` writes a structured record — who did what
/// to which resource, and how it ended — through `ash-log`'s audit backend.
/// Registering it is left to you, because where those records go is a
/// deployment decision the crate should not make.
///
/// Wired the same way either one is: `Hooks::new().with(…)`.
pub struct Trace;

impl Hook for Trace {
    fn after(&self, event: &Event<'_>, outcome: &Outcome, ctx: &Context) {
        println!(
            "  [hook] {:<6} {}/{:<4} -> {} (principal: {}, correlation: {})",
            event.operation.as_str(),
            event.resource,
            event.id.unwrap_or("-"),
            outcome.status(),
            ctx.principal().unwrap_or("anonymous"),
            ctx.correlation_id(),
        );
    }
}
