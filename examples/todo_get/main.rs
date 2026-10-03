//! The smallest useful JSON:API service: `GET /todos` and `GET /todos/{id}`.
//!
//! **Standalone** — no `ash-domain`, no database, no `Cargo.toml` feature to
//! turn on beyond the defaults.
//!
//! Read-only, so it implements exactly two traits. There is no `Create`,
//! `Update` or `Delete` impl anywhere in this example — one trait per
//! operation means a service that only reads never writes an unused method,
//! and a `POST` here answers `405` because nothing routed it.
//!
//! # Layout
//!
//! Three files, split along the seam the crate itself draws:
//!
//! - [`todo`] — **the JSON:API**. What a `Todo` is on the wire, and the two
//!   read operations. Mentions no router, port or request.
//! - [`store`] — **the data**. Rows in memory. Mentions neither JSON:API nor
//!   axum, and is what you would swap for a database.
//! - [`http`] — **the axum**. Routes, the extra non-JSON:API endpoint, and
//!   the server.
//!
//! The dependency arrow runs one way — `http` → `todo` → `store` — so the
//! transport can change without touching the resource, and the storage can
//! change without touching either.
//!
//! ```text
//! cargo run --example todo_get
//! ```
//!
//! Then, in another terminal:
//!
//! ```text
//! curl -s localhost:3000/todos | jq
//! curl -s 'localhost:3000/todos?page[limit]=2&sort=-message' | jq
//! curl -s localhost:3000/todos/t_1 | jq
//! curl -si localhost:3000/todos/nope          # 404, JSON:API shaped
//! curl -s -H 'Accept-Language: de' localhost:3000/todos/nope | jq
//! curl -s localhost:3000/health               # plain text, not JSON:API
//! ```

mod attachment;
mod http;
mod store;
mod todo;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    http::serve(store::App::seeded()).await?;
    Ok(())
}
