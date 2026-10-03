//! The middleware every generated handler needs.
//!
//! [`api!`](crate::api!)'s handlers take the request's [`Context`] as an axum
//! extension. Something has to put it there, and that is [`layer`]: it reads
//! the client's request id, generates one when there is none, and attaches
//! the context.
//!
//! ```ignore
//! let app = routes()
//!     .layer(ash_jsonapi::layer())
//!     .with_state(state);
//! ```
//!
//! Without it, every request fails with axum's "Missing request extension"
//! `500` — the handlers cannot build a context themselves, because what goes
//! into one (the principal, the tenant) is decided by middleware that runs
//! before them.
//!
//! # Adding your own
//!
//! Authentication runs after this and adds to what it built:
//!
//! ```ignore
//! async fn authenticate(mut req: Request, next: Next) -> Response {
//!     let ctx = req
//!         .extensions()
//!         .get::<Context>()
//!         .expect("ash_jsonapi::layer() runs first")
//!         .clone()
//!         .with_principal("alice@example.com");
//!
//!     req.extensions_mut().insert(ctx);
//!     next.run(req).await
//! }
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::Request;
use axum::middleware::{Next, from_fn};
use axum::response::Response;

use crate::context::Context;

/// The header a correlation id is read from and echoed back on.
pub const REQUEST_ID: &str = "x-request-id";

/// The header the response locale is negotiated from.
pub const ACCEPT_LANGUAGE: &str = "accept-language";

/// Attach a [`Context`] to every request.
///
/// The one piece of wiring [`api!`](crate::api!) requires:
///
/// ```ignore
/// let app = routes()
///     .layer(ash_jsonapi::layer())
///     .with_state(state);
/// ```
///
/// The correlation id is the client's `X-Request-Id` where it sent one, and
/// a generated id otherwise. Either way it is echoed back on the response, so
/// a client can quote it, and it is the `id` on every error object this crate
/// returns — the thread tying a failed response to the log line explaining
/// it.
///
/// Add authentication *after* this layer and extend the context it built,
/// rather than replacing it — see the [module docs](self).
pub fn layer()
-> axum::middleware::FromFnLayer<impl FnMut(Request, Next) -> WithContext + Clone, (), (Request,)> {
    // The last type parameter is axum's argument list *before* `Next`, so a
    // middleware taking the whole request is the 1-tuple `(Request,)`.
    from_fn(|request: Request, next: Next| with_context(request, next))
}

/// The future [`layer`]'s middleware returns.
pub type WithContext = std::pin::Pin<Box<dyn Future<Output = Response> + Send>>;

/// Read or generate the correlation id, attach the context, echo the id back.
fn with_context(request: Request, next: Next) -> WithContext {
    Box::pin(run(request, next))
}

async fn run(mut request: Request, next: Next) -> Response {
    let id = request
        .headers()
        .get(REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        // A client-supplied id reaches the logs, so it is bounded and
        // filtered: an unbounded or newline-bearing header would let a
        // caller forge log entries.
        .filter(|id| !id.is_empty() && id.len() <= 128 && id.chars().all(is_safe))
        .map(ToString::to_string)
        .unwrap_or_else(request_id);

    // The locale the client asked for, recorded on the context so a handler
    // can consult it and the response can say what it negotiated. An absent
    // or unparsable header simply leaves it unset.
    let locale = request
        .headers()
        .get(ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .and_then(crate::i18n::Locale::negotiate);

    let ctx = match locale {
        Some(locale) => Context::new(&id).with_locale(locale),
        None => Context::new(&id),
    };

    request.extensions_mut().insert(ctx);

    let mut response = next.run(request).await;

    // Echoed back so a client can quote the id when reporting a failure.
    if let Ok(value) = id.parse() {
        response.headers_mut().insert(REQUEST_ID, value);
    }

    response
}

/// Whether a character may appear in a client-supplied correlation id.
///
/// Conservative on purpose: this string is written to logs.
fn is_safe(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

/// A correlation id for a request that arrived without one.
///
/// Enough to correlate one request's log lines within a deployment: the
/// process start time, and a counter. Not a UUID and not unguessable — if you
/// need either, set `X-Request-Id` at your proxy, which is where a
/// distributed trace id comes from anyway.
fn request_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    // Read once per process: two processes starting in the same microsecond
    // is the collision this does not defend against.
    static START: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let start = *START.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_micros() as u64)
            .unwrap_or(0)
    });

    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{start:x}-{n:x}")
}
