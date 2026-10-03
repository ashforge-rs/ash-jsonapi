//! The axum half: routes, and the server that serves them.
//!
//! This is where the transport lives. [`crate::todo`] said what a `Todo` is
//! and how to fetch one; this file decides that it is reachable over HTTP,
//! on this port, under these verbs.
//!
//! `api!` generates the handlers — one per operation named in `operations`,
//! and nothing for the rest, which is what makes the operation traits
//! opt-in. Everything below is ordinary axum: `routes()` is an
//! `axum::Router<App>` like any other, so a health endpoint or your own
//! middleware merges into it the usual way.

use crate::attachment::Attachment;
use crate::store::App;
use crate::todo::{BASE_PATH, Todo};

// The routes, and only these two. `LIST` and `READ` are the operations whose
// traits are implemented in `todo` — naming `CREATE` here would not compile,
// which is the point of one trait per operation.
ash_jsonapi::api! {
    operations: [ash_jsonapi::LIST, ash_jsonapi::STREAM, ash_jsonapi::READ],
    resource: Todo,
    name: "todo",
    path: "/todos",
    state: App,
}

// A file resource: upload it, read its metadata, download the bytes.
// `CONTENT` is what routes `/attachments/{id}/content`; without it the bytes
// have no URL and the trait is never required.
mod files {
    use super::*;

    ash_jsonapi::api! {
        operations: [ash_jsonapi::CREATE, ash_jsonapi::READ, ash_jsonapi::CONTENT],
        resource: Attachment,
        name: "attachment",
        path: "/attachments",
        state: App,
    }
}

/// The whole service: the JSON:API routes, plus anything else this server
/// happens to expose.
///
/// `router!` merges the generated routes and attaches the context layer. The
/// layer is not optional: without it no handler can find its `Context`, and
/// every request is a `500`.
pub fn app(state: App) -> axum::Router {
    ash_jsonapi::router!(App, routes(), files::routes())
        // Ordinary axum from here. Health sits outside the JSON:API surface
        // — it answers `text/plain`, not `application/vnd.api+json`, so it
        // is a plain route rather than a resource.
        .route("/health", axum::routing::get(|| async { "ok" }))
        .with_state(state)
}

/// Bind, serve, and drain in-flight requests on shutdown.
pub async fn serve(state: App) -> std::io::Result<()> {
    println!("listening on http://{ADDR}");
    println!("  GET {BASE_PATH}");
    println!("  GET {BASE_PATH}/{{id}}");
    println!("  POST /attachments          (raw bytes)");
    println!("  GET  /attachments/{{id}}");
    println!("  GET  /attachments/{{id}}/content");
    println!("  GET  {BASE_PATH}  (Accept: application/jsonl streams)");
    println!("  GET  /health");

    // Binds, serves, and drains in-flight requests on SIGTERM — the signal a
    // deploy actually sends.
    ash_jsonapi::serve!(ADDR, app(state)).await
}

/// Where the server listens.
const ADDR: &str = "0.0.0.0:3000";
