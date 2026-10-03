//! `before` and `after` — one interception point for every operation.
//!
//! A hook runs around an operation without the operation knowing. That is
//! what authorization, audit, metrics and tracing all want, and building
//! them on one mechanism is why this module exists: four separate
//! interception points would be four places to forget, four orderings to
//! reason about, and four things to test.
//!
//! ```
//! use ash_jsonapi::hook::{Event, Hook, Outcome};
//! use ash_jsonapi::{Context, JsonApiError};
//!
//! struct Timing;
//!
//! impl Hook for Timing {
//!     fn after(&self, event: &Event<'_>, outcome: &Outcome, _ctx: &Context) {
//!         println!(
//!             "{} {} -> {}",
//!             event.operation.as_str(),
//!             event.resource,
//!             outcome.status()
//!         );
//!     }
//! }
//! # let _ = Timing;
//! ```
//!
//! # What a `before` hook can do
//!
//! Return an error, and the operation does not run: the request is answered
//! with what the hook returned. That makes `before` the seam for
//! authorization — a denial should never reach storage — and for anything
//! else that decides a request must not proceed.
//!
//! An `after` hook cannot change the response. It observes: it is told what
//! happened, and its return value is nothing. Recording must not be able to
//! break a request that otherwise succeeded, and a hook that could rewrite a
//! response is a second, invisible handler.
//!
//! # Ordering
//!
//! `before` hooks run in registration order, and the first error stops the
//! rest — so a hook registered later can rely on an earlier one having
//! passed. `after` hooks run in the same order, and all of them run: one
//! panicking or misbehaving must not silence the others' records.

use crate::context::Context;
use crate::error::JsonApiError;

/// Which operation is running.
///
/// The five JSON:API operations, named as [`api!`](crate::api!) names them —
/// so a hook's `match` reads like the `operations` list that routed them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// `GET` on the collection.
    List,
    /// `POST` on the collection.
    Create,
    /// `GET` on one resource.
    Read,
    /// `PATCH` on one resource.
    Update,
    /// `DELETE` on one resource.
    Delete,
}

impl Operation {
    /// The lowercase name, for a log line or a metric label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Create => "create",
            Self::Read => "read",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }

    /// Whether this operation changes state.
    ///
    /// The distinction an audit trail cares about most: a read that is
    /// denied is worth recording, and a *write* that is denied is worth
    /// alerting on.
    pub fn is_write(self) -> bool {
        matches!(self, Self::Create | Self::Update | Self::Delete)
    }
}

/// What is about to run, or has just run.
///
/// Borrowed rather than owned: a hook is called on the request path, and
/// copying a resource name per hook per request is a cost with no purpose.
#[derive(Debug, Clone, Copy)]
pub struct Event<'a> {
    /// The operation.
    pub operation: Operation,
    /// The resource's JSON:API `type`, e.g. `"todo"`.
    pub resource: &'a str,
    /// The id the request addressed, for the operations that name one.
    ///
    /// `None` for [`List`](Operation::List) and
    /// [`Create`](Operation::Create), which address the collection.
    pub id: Option<&'a str>,
}

/// How an operation finished.
///
/// Handed to [`Hook::after`], which cannot change it — see the module docs.
#[derive(Debug, Clone, Copy)]
pub enum Outcome {
    /// The operation ran and the response is being written.
    Succeeded {
        /// The status the client is being handed: `201` for a create, `204`
        /// for a delete, `200` otherwise.
        ///
        /// Carried rather than assumed, so an audit record says what the
        /// client actually saw.
        status: u16,
    },
    /// The operation failed, or a `before` hook refused it.
    Failed {
        /// The status the client is being handed.
        status: u16,
    },
}

impl Outcome {
    /// The HTTP status the client receives.
    pub fn status(&self) -> u16 {
        match self {
            Self::Succeeded { status } | Self::Failed { status } => *status,
        }
    }

    /// Whether the operation ran to completion.
    pub fn succeeded(&self) -> bool {
        matches!(self, Self::Succeeded { .. })
    }
}

/// Something that runs around every operation.
///
/// Both methods have a default that does nothing, so a hook implements only
/// the half it cares about: an audit sink is an `after`, an authorization
/// check is a `before`.
pub trait Hook: Send + Sync + 'static {
    /// Runs before the operation. An `Err` refuses the request, and the
    /// operation never runs.
    ///
    /// This is where authorization belongs: a denial that reaches storage
    /// has already leaked whether the row exists.
    fn before(&self, _event: &Event<'_>, _ctx: &Context) -> Result<(), JsonApiError> {
        Ok(())
    }

    /// Runs after the operation, whatever happened.
    ///
    /// Cannot change the response — recording must not be able to break a
    /// request that otherwise worked.
    fn after(&self, _event: &Event<'_>, _outcome: &Outcome, _ctx: &Context) {}
}

/// The hooks a service runs, in order.
///
/// Built once at startup and held on your state. Empty by default, and an
/// empty set is the zero-cost case: the generated handlers skip the whole
/// mechanism when there is nothing registered.
#[derive(Default, Clone)]
pub struct Hooks(std::sync::Arc<Vec<Box<dyn Hook>>>);

impl Hooks {
    /// No hooks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a hook, after the ones already registered.
    #[must_use]
    pub fn with(mut self, hook: impl Hook) -> Self {
        // `Arc::make_mut` needs `Clone`; building the set is a startup-time
        // operation, so taking the vec out and putting it back is fine and
        // keeps `Hooks` cheap to clone per request.
        let mut hooks = std::mem::take(&mut self.0);
        let vec =
            std::sync::Arc::get_mut(&mut hooks).expect("hooks are built before they are shared");
        vec.push(Box::new(hook));
        Self(hooks)
    }

    /// Whether any hook is registered.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Run every `before` hook, stopping at the first refusal.
    pub fn before(&self, event: &Event<'_>, ctx: &Context) -> Result<(), JsonApiError> {
        for hook in self.0.iter() {
            hook.before(event, ctx)?;
        }
        Ok(())
    }

    /// Run every `after` hook. All of them run.
    pub fn after(&self, event: &Event<'_>, outcome: &Outcome, ctx: &Context) {
        for hook in self.0.iter() {
            hook.after(event, outcome, ctx);
        }
    }
}

impl std::fmt::Debug for Hooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The hooks themselves are opaque; the count is what is useful.
        f.debug_struct("Hooks")
            .field("count", &self.0.len())
            .finish()
    }
}
