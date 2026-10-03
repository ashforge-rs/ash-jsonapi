//! The axum half: routes, authentication, and the server.
//!
//! Two resources are routed here rather than one, which is the only thing
//! that changes when a service grows: each gets its own `api!` in its own
//! module — because both expand to a function called `routes()` — and
//! `router!` merges them.
//!
//! This is also where the principal comes from. The crate's layer builds the
//! [`Context`]; deciding *who* is making the request is your service's
//! business, so it is an ordinary axum middleware here.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

use ash_jsonapi::Context;

use crate::blog::{AUTHORS_PATH, Author, POSTS_PATH, Post};
use crate::store::App;

/// The posts routes: the full CRUD surface.
///
/// `CRUD` is shorthand for all five operations. Spelled out here as the
/// individual names, because that is the form you edit when a service should
/// stop accepting deletes — and each name requires its trait, so removing one
/// removes a compile error rather than leaving dead code.
mod posts {
    use super::*;

    ash_jsonapi::api! {
        operations: [
            ash_jsonapi::LIST,
            ash_jsonapi::CREATE,
            ash_jsonapi::READ,
            ash_jsonapi::UPDATE,
            ash_jsonapi::DELETE
        ],
        resource: Post,
        name: "post",
        path: "/posts",
        state: App,
    }
}

/// The authors routes: read and create only.
///
/// No `UPDATE`, no `DELETE` — and correspondingly no `Update` or `Delete`
/// impl in [`crate::blog`]. A `DELETE /authors/a_1` is a `405`, decided by
/// the router rather than by a handler that then refuses.
mod authors {
    use super::*;

    ash_jsonapi::api! {
        operations: [ash_jsonapi::LIST, ash_jsonapi::CREATE, ash_jsonapi::READ],
        resource: Author,
        name: "author",
        path: "/authors",
        state: App,
    }
}

/// Read the principal from a header and add it to the request's context.
///
/// A stand-in for real authentication — a session cookie, a bearer token,
/// mTLS — kept to one header so the demo can toggle it with a `curl` flag.
/// What matters is the shape: it *extends* the context the crate's layer
/// built rather than replacing it, so the correlation id and negotiated
/// locale survive.
///
/// [`crate::hooks::RequireAuthForWrites`] is what then reads the principal.
/// The two are deliberately separate: this decides who you are, the hook
/// decides what that lets you do.
async fn authenticate(mut request: Request, next: Next) -> Response {
    let principal = request
        .headers()
        .get("x-principal")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    if let Some(principal) = principal {
        // `layer()` runs first, so the context is already there. Extending
        // the existing one keeps the correlation id that the response will
        // be tagged with.
        if let Some(ctx) = request.extensions().get::<Context>().cloned() {
            request
                .extensions_mut()
                .insert(ctx.with_principal(principal));
        }
    }

    next.run(request).await
}

/// The whole service.
///
/// Written out rather than with `router!`, because this service needs its own
/// middleware between the routes and the context layer — which is exactly the
/// case the macro's docs say to expand.
///
/// Order matters and reads bottom-up: `layer()` runs first and builds the
/// context, then `authenticate` adds the principal to it, then the handlers
/// run and the hooks fire.
pub fn app(state: App) -> axum::Router {
    axum::Router::new()
        .merge(posts::routes())
        .merge(authors::routes())
        .layer(axum::middleware::from_fn(authenticate))
        .layer(ash_jsonapi::layer())
        .with_state(state)
}

/// Bind, serve, and drain in-flight requests on shutdown.
pub async fn serve(state: App) -> std::io::Result<()> {
    println!("listening on http://{ADDR}");
    println!();
    println!("  GET    {POSTS_PATH}");
    println!("  POST   {POSTS_PATH}         (write: needs X-Principal)");
    println!("  GET    {POSTS_PATH}/{{id}}");
    println!("  PATCH  {POSTS_PATH}/{{id}}    (write: needs X-Principal)");
    println!("  DELETE {POSTS_PATH}/{{id}}    (write: needs X-Principal)");
    println!("  GET    {AUTHORS_PATH}");
    println!("  POST   {AUTHORS_PATH}       (write: needs X-Principal)");
    println!();

    ash_jsonapi::serve!(ADDR, app(state)).await
}

/// Where the server listens. A different port from `todo_get`, so both
/// examples can run at once.
const ADDR: &str = "0.0.0.0:3001";
