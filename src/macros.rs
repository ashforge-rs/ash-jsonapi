//! Call-site conveniences: the one-liners a handler is mostly made of.
//!
//! Everything here is sugar over [`JsonApiError`](crate::JsonApiError),
//! [`api!`](crate::api!) and axum — there is no behaviour in this module that
//! is not reachable by writing the long form, and the long form stays the
//! documented one. What these buy is that the short form is *also* the
//! correct one: an error built by hand is one forgotten `.in_context(ctx)`
//! away from an error the audit log cannot resolve, and a router assembled by
//! hand is one forgotten [`layer()`](crate::layer()) away from a service where
//! every request is a `500`.
//!
//! # Errors
//!
//! [`jsonapi_error!`](crate::jsonapi_error!) builds any code, and the named
//! shortcuts cover the ones a handler actually reaches for:
//!
//! ```
//! # use ash_jsonapi::Context;
//! # let ctx = Context::new("");
//! # let field = "message";
//! // Long form:
//! # use ash_jsonapi::{ErrorCode, JsonApiError};
//! let err = JsonApiError::in_request(ErrorCode::NotFound, &ctx)
//!     .detail("no such todo");
//!
//! // Short form — the same error, with the context threaded for you:
//! let err = ash_jsonapi::not_found!(ctx, "no such todo");
//!
//! // Pointing at the member at fault:
//! let err = ash_jsonapi::invalid_attribute!(ctx, "must not be empty", field);
//! # let _ = err;
//! ```
//!
//! # Guards
//!
//! [`ensure!`](crate::ensure!) is the `rpc_validate!` of this crate: a
//! precondition that returns a JSON:API error rather than panicking, so a
//! handler reads as its rules.
//!
//! ```
//! # use ash_jsonapi::{Context, JsonApiError};
//! fn check(message: &str, ctx: &Context) -> Result<(), JsonApiError> {
//!     ash_jsonapi::ensure!(!message.is_empty(), ctx, "message must not be empty", "message");
//!     Ok(())
//! }
//! # let ctx = Context::new("");
//! # assert!(check("", &ctx).is_err());
//! # assert!(check("hi", &ctx).is_ok());
//! ```

/// Build a [`JsonApiError`](crate::JsonApiError) of any code, in a request's
/// context.
///
/// The general form the named shortcuts below are written in terms of. The
/// context comes first because every error should carry the correlation id —
/// that is what makes "here is my error id" resolvable in the audit log, and
/// threading it here means it cannot be forgotten at a call site.
///
/// ```
/// # use ash_jsonapi::{Context, ErrorCode};
/// # let ctx = Context::new("");
/// // A bare code:
/// let err = ash_jsonapi::jsonapi_error!(ctx, ErrorCode::Conflict);
///
/// // With client-facing detail:
/// let err = ash_jsonapi::jsonapi_error!(ctx, ErrorCode::Conflict, "the row moved under you");
///
/// // Pointing at an attribute, or a query parameter:
/// let err = ash_jsonapi::jsonapi_error!(ctx, ErrorCode::InvalidAttribute, "too long", attribute: "message");
/// let err = ash_jsonapi::jsonapi_error!(ctx, ErrorCode::InvalidParameter, "unknown field", parameter: "sort");
/// # let _ = err;
/// ```
#[macro_export]
macro_rules! jsonapi_error {
    ($ctx:expr, $code:expr $(,)?) => {
        $crate::JsonApiError::in_request($code, &$ctx)
    };
    ($ctx:expr, $code:expr, $detail:expr $(,)?) => {
        $crate::JsonApiError::in_request($code, &$ctx).detail($detail)
    };
    ($ctx:expr, $code:expr, $detail:expr, attribute: $field:expr $(,)?) => {
        $crate::JsonApiError::in_request($code, &$ctx)
            .detail($detail)
            .attribute($field)
    };
    ($ctx:expr, $code:expr, $detail:expr, pointer: $pointer:expr $(,)?) => {
        $crate::JsonApiError::in_request($code, &$ctx)
            .detail($detail)
            .pointer($pointer)
    };
    ($ctx:expr, $code:expr, $detail:expr, parameter: $param:expr $(,)?) => {
        $crate::JsonApiError::in_request($code, &$ctx)
            .detail($detail)
            .parameter($param)
    };
}

/// A `404`, optionally saying what was not found.
///
/// ```
/// # use ash_jsonapi::Context;
/// # let ctx = Context::new("");
/// let err = ash_jsonapi::not_found!(ctx);
/// let err = ash_jsonapi::not_found!(ctx, "no todo with that id");
/// # let _ = err;
/// ```
#[macro_export]
macro_rules! not_found {
    ($ctx:expr $(,)?) => {
        $crate::jsonapi_error!($ctx, $crate::ErrorCode::NotFound)
    };
    ($ctx:expr, $detail:expr $(,)?) => {
        $crate::jsonapi_error!($ctx, $crate::ErrorCode::NotFound, $detail)
    };
}

/// A `403` — a deliberate denial, not an outage.
#[macro_export]
macro_rules! forbidden {
    ($ctx:expr $(,)?) => {
        $crate::jsonapi_error!($ctx, $crate::ErrorCode::Forbidden)
    };
    ($ctx:expr, $detail:expr $(,)?) => {
        $crate::jsonapi_error!($ctx, $crate::ErrorCode::Forbidden, $detail)
    };
}

/// A `409` — an optimistic-concurrency failure the client may retry.
#[macro_export]
macro_rules! conflict {
    ($ctx:expr $(,)?) => {
        $crate::jsonapi_error!($ctx, $crate::ErrorCode::Conflict)
    };
    ($ctx:expr, $detail:expr $(,)?) => {
        $crate::jsonapi_error!($ctx, $crate::ErrorCode::Conflict, $detail)
    };
}

/// A `422` naming the attribute at fault.
///
/// The third argument is an attribute *name*, not a pointer: `message`
/// becomes `/data/attributes/message`, and a dotted `address.city` nests.
/// Omit it when no single member is to blame.
///
/// ```
/// # use ash_jsonapi::Context;
/// # let ctx = Context::new("");
/// let err = ash_jsonapi::invalid_attribute!(ctx, "must not be empty", "message");
/// let err = ash_jsonapi::invalid_attribute!(ctx, "one of `from`/`to` is required");
/// # let _ = err;
/// ```
#[macro_export]
macro_rules! invalid_attribute {
    ($ctx:expr, $detail:expr $(,)?) => {
        $crate::jsonapi_error!($ctx, $crate::ErrorCode::InvalidAttribute, $detail)
    };
    ($ctx:expr, $detail:expr, $field:expr $(,)?) => {
        $crate::jsonapi_error!(
            $ctx,
            $crate::ErrorCode::InvalidAttribute,
            $detail,
            attribute: $field
        )
    };
}

/// A `400` naming the query parameter at fault.
///
/// ```
/// # use ash_jsonapi::Context;
/// # let ctx = Context::new("");
/// let err = ash_jsonapi::invalid_parameter!(ctx, "cannot sort on `secret`", "sort");
/// # let _ = err;
/// ```
#[macro_export]
macro_rules! invalid_parameter {
    ($ctx:expr, $detail:expr $(,)?) => {
        $crate::jsonapi_error!($ctx, $crate::ErrorCode::InvalidParameter, $detail)
    };
    ($ctx:expr, $detail:expr, $param:expr $(,)?) => {
        $crate::jsonapi_error!(
            $ctx,
            $crate::ErrorCode::InvalidParameter,
            $detail,
            parameter: $param
        )
    };
}

/// A `500` that says nothing to the client.
///
/// There is no detail argument, and that is deliberate: the detail of an
/// internal error is exactly what must not reach a client, and
/// [`ErrorCode::is_sanitized`](crate::ErrorCode::is_sanitized) would drop it
/// anyway. Log the cause against the correlation id this carries.
#[macro_export]
macro_rules! internal_error {
    ($ctx:expr $(,)?) => {
        $crate::JsonApiError::internal(&$ctx)
    };
}

/// Return a JSON:API error unless a condition holds.
///
/// The guard clause a handler's preconditions are written as — the
/// counterpart of `assert!` for something that is the client's fault rather
/// than a bug. Everything after the condition is passed to
/// [`invalid_attribute!`](crate::invalid_attribute!), so the member at fault
/// is named where there is one.
///
/// ```
/// # use ash_jsonapi::{Context, JsonApiError};
/// fn rename(name: &str, ctx: &Context) -> Result<(), JsonApiError> {
///     ash_jsonapi::ensure!(!name.is_empty(), ctx, "name must not be empty", "name");
///     ash_jsonapi::ensure!(name.len() <= 80, ctx, "name must be 80 characters or fewer", "name");
///     Ok(())
/// }
/// # let ctx = Context::new("");
/// # assert!(rename("", &ctx).is_err());
/// # assert!(rename("ok", &ctx).is_ok());
/// ```
///
/// Use [`ensure_code!`](crate::ensure_code!) for a guard that is not a `422`.
#[macro_export]
macro_rules! ensure {
    ($condition:expr, $ctx:expr, $($error:tt)+) => {
        if !($condition) {
            return ::std::result::Result::Err(
                $crate::invalid_attribute!($ctx, $($error)+).into()
            );
        }
    };
}

/// Return an error of a chosen code unless a condition holds.
///
/// [`ensure!`](crate::ensure!) with the code spelled out — for a guard that
/// is a denial, a conflict, or anything else a `422` would misdescribe.
///
/// ```
/// # use ash_jsonapi::{Context, ErrorCode, JsonApiError};
/// fn delete(owner: &str, actor: &str, ctx: &Context) -> Result<(), JsonApiError> {
///     ash_jsonapi::ensure_code!(
///         owner == actor,
///         ctx,
///         ErrorCode::Forbidden,
///         "only the owner may delete this todo"
///     );
///     Ok(())
/// }
/// # let ctx = Context::new("");
/// # assert!(delete("a", "b", &ctx).is_err());
/// # assert!(delete("a", "a", &ctx).is_ok());
/// ```
#[macro_export]
macro_rules! ensure_code {
    ($condition:expr, $ctx:expr, $($error:tt)+) => {
        if !($condition) {
            return ::std::result::Result::Err(
                $crate::jsonapi_error!($ctx, $($error)+).into()
            );
        }
    };
}

/// Unwrap an [`Option`], or return a `404`.
///
/// The shape of nearly every `READ` handler: a row that is not there is not
/// an error the server made, it is a `404` with a sentence.
///
/// ```
/// # use ash_jsonapi::{Context, JsonApiError};
/// # struct Todo;
/// fn fetch(row: Option<Todo>, ctx: &Context) -> Result<Todo, JsonApiError> {
///     let todo = ash_jsonapi::found!(row, ctx, "no todo with that id");
///     Ok(todo)
/// }
/// # let ctx = Context::new("");
/// # assert!(fetch(None, &ctx).is_err());
/// # assert!(fetch(Some(Todo), &ctx).is_ok());
/// ```
#[macro_export]
macro_rules! found {
    ($option:expr, $ctx:expr $(,)?) => {
        match $option {
            ::std::option::Option::Some(value) => value,
            ::std::option::Option::None => {
                return ::std::result::Result::Err($crate::not_found!($ctx).into());
            }
        }
    };
    ($option:expr, $ctx:expr, $detail:expr $(,)?) => {
        match $option {
            ::std::option::Option::Some(value) => value,
            ::std::option::Option::None => {
                return ::std::result::Result::Err($crate::not_found!($ctx, $detail).into());
            }
        }
    };
}

/// A [`Translation`](crate::i18n::Translation) key with its params.
///
/// The short form of `Translation::new(key).param(k, v)…`, for attaching to
/// an error with [`translate`](crate::JsonApiError::translate).
///
/// ```
/// # use ash_jsonapi::Context;
/// # let ctx = Context::new("");
/// let err = ash_jsonapi::invalid_attribute!(ctx, "must be 80 or fewer", "name")
///     .translate(ash_jsonapi::t!("todo.name.too_long", max = 80));
///
/// let meta = err.to_object().meta.unwrap();
/// assert_eq!(meta["i18n"]["key"], "todo.name.too_long");
/// assert_eq!(meta["i18n"]["params"]["max"], 80);
/// ```
#[macro_export]
macro_rules! t {
    ($key:expr $(,)?) => {
        $crate::i18n::Translation::new($key)
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::Translation::new($key)
            $(.param(::std::stringify!($name), $value))+
    };
}

/// Assemble a router from several `api!` modules, with the context layer
/// already attached.
///
/// [`api!`](crate::api!) writes a `routes()` per resource, which leaves two
/// things to remember at the top level: merging them, and calling
/// [`layer()`](crate::layer()). Forgetting the second is not a subtle bug —
/// every request becomes a `500`, because no handler can find its
/// [`Context`](crate::Context) — so it is worth not being able to forget.
///
/// ```ignore
/// mod todos { ash_jsonapi::api! { /* … */ } }
/// mod users { ash_jsonapi::api! { /* … */ } }
///
/// let app = ash_jsonapi::router!(App, todos::routes(), users::routes());
/// ```
///
/// Equivalent to merging each router, adding `layer()`, and calling
/// `.with_state(…)` yourself — write that out when you need another layer in
/// between.
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
#[macro_export]
macro_rules! router {
    ($state:ty $(, $routes:expr)+ $(,)?) => {{
        ::axum::Router::<$state>::new()
            $(.merge($routes))+
            .layer($crate::layer())
    }};
}

/// Bind an address and serve, until the process is asked to stop.
///
/// The last step of every service, and the one with the most ways to be
/// subtly wrong: a server that ignores `SIGTERM` is killed mid-request on
/// every deploy, so the shutdown signal is wired in rather than left as an
/// exercise.
///
/// ```ignore
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let app = ash_jsonapi::router!(App, todos::routes()).with_state(state);
///     ash_jsonapi::serve!("0.0.0.0:3000", app).await?;
///     Ok(())
/// }
/// ```
///
/// Yields `std::io::Result<()>`. Use [`axum::serve()`] directly when you need
/// a listener you built yourself, or a different shutdown condition.
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
#[macro_export]
macro_rules! serve {
    ($addr:expr, $app:expr $(,)?) => {
        async {
            let listener = ::tokio::net::TcpListener::bind($addr).await?;
            ::axum::serve(listener, $app)
                .with_graceful_shutdown($crate::shutdown_signal())
                .await
        }
    };
}

/// Resolve when the process is asked to stop: `Ctrl-C`, or `SIGTERM`.
///
/// What [`serve!`](crate::serve!) waits on, exposed because a service that
/// builds its own [`axum::serve()`] wants the same condition. `SIGTERM` is the
/// one that matters in production — it is what a container runtime and an
/// orchestrator send before a deploy — and it is the one most hand-written
/// shutdown handlers omit, leaving in-flight requests to be killed rather
/// than finished.
///
/// On a non-Unix target only `Ctrl-C` is awaited; there is no `SIGTERM` to
/// listen for.
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
pub async fn shutdown_signal() {
    let interrupt = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install the Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install the SIGTERM handler")
            .recv()
            .await;
    };

    // Nothing to wait for off Unix; `pending()` keeps the `select!` below one
    // shape rather than two.
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}
