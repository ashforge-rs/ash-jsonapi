//! Writes, relationships, validation and hooks: the half of the crate that
//! `todo_get` does not reach.
//!
//! **Standalone** — no `ash-domain`, no database. Two related resources,
//! `Post` and `Author`, with the full write surface on one of them.
//!
//! Where `todo_get` shows how to serve a resource, this shows what happens
//! when clients start sending bodies:
//!
//! - **Writes** — `POST`, `PATCH` and `DELETE`, one trait each, so a service
//!   implements only what it routes.
//! - **Relationships** — a to-one (`author`) and a to-many (`tags`),
//!   declared once and reaching linkage, links and the request schema.
//! - **Validation** — bodies checked against a JSON Schema derived from the
//!   resource declaration, before anything reaches storage. Failures come
//!   back as `422`s with a JSON Pointer at the offending member.
//! - **Error classification** — the difference between a `422` (this body is
//!   wrong) and a `409` (this body conflicts with what is stored), which
//!   only shows up once there are writes to conflict.
//! - **Hooks** — one `before`/`after` pair seeing every operation on every
//!   resource: authorization that cannot be forgotten per-handler, and the
//!   shape [`ash_jsonapi::audit::Audit`] plugs into.
//!
//! # Layout
//!
//! The same three-way split as `todo_get`, plus the hooks:
//!
//! - [`blog`] — **the JSON:API**. The two resources and their operations.
//! - [`store`] — **the data**. Rows in memory, and the invariants writes
//!   must hold. Mentions neither JSON:API nor axum.
//! - [`hooks`] — **the policy**. Cross-cutting rules, blind to which
//!   resource they are guarding.
//! - [`http`] — **the axum**. Routes, authentication, and the server.
//!
//! ```text
//! cargo run --example blog
//! ```
//!
//! Then, in another terminal. Reads work as they do anywhere:
//!
//! ```text
//! curl -s localhost:3001/posts | jq
//! curl -s 'localhost:3001/posts?sort=-title&page[limit]=1' | jq
//! curl -s localhost:3001/posts/p_1 | jq        # note `relationships`
//! ```
//!
//! Writes are refused without a principal — the `before` hook stops them
//! before storage is touched:
//!
//! ```text
//! curl -si -X DELETE localhost:3001/posts/p_2  # 403
//! ```
//!
//! With one, the same request succeeds:
//!
//! ```text
//! curl -si -X DELETE -H 'X-Principal: ada' localhost:3001/posts/p_2   # 204
//! ```
//!
//! A valid create, with both relationships:
//!
//! ```text
//! curl -s -X POST localhost:3001/posts \
//!   -H 'Content-Type: application/vnd.api+json' \
//!   -H 'X-Principal: ada' \
//!   -d '{"data":{"type":"post","attributes":{"title":"Third","slug":"third",
//!        "body":"...","published":false},"relationships":{
//!        "author":{"data":{"type":"author","id":"a_1"}},
//!        "tags":{"data":[{"type":"tag","id":"t_rust"}]}}}}' | jq
//! ```
//!
//! The three ways it can fail, each a different status. A body that is wrong
//! in itself never reaches storage — `422`, with a pointer at the member:
//!
//! ```text
//! curl -s -X POST localhost:3001/posts \
//!   -H 'Content-Type: application/vnd.api+json' -H 'X-Principal: ada' \
//!   -d '{"data":{"type":"post","attributes":{"title":"No slug"}}}' | jq
//! ```
//!
//! A body with no `author` linkage at all — `422`, pointing at the
//! relationship rather than at an id that was never sent:
//!
//! ```text
//! curl -s -X POST localhost:3001/posts \
//!   -H 'Content-Type: application/vnd.api+json' -H 'X-Principal: ada' \
//!   -d '{"data":{"type":"post","attributes":{"title":"Orphan","slug":"orphan",
//!        "body":"...","published":false}}}' | jq
//! ```
//!
//! A body that is well-formed but names an author that does not exist —
//! `422`, pointing at the id:
//!
//! ```text
//! curl -s -X POST localhost:3001/posts \
//!   -H 'Content-Type: application/vnd.api+json' -H 'X-Principal: ada' \
//!   -d '{"data":{"type":"post","attributes":{"title":"Ghost","slug":"ghost",
//!        "body":"...","published":false},"relationships":{
//!        "author":{"data":{"type":"author","id":"a_99"}}}}}' | jq
//! ```
//!
//! And one that would have been accepted a moment earlier — `409`:
//!
//! ```text
//! curl -s -X POST localhost:3001/posts \
//!   -H 'Content-Type: application/vnd.api+json' -H 'X-Principal: ada' \
//!   -d '{"data":{"type":"post","attributes":{"title":"Dup","slug":"hello",
//!        "body":"...","published":false},"relationships":{
//!        "author":{"data":{"type":"author","id":"a_1"}}}}}' | jq
//! ```
//!
//! An operation nobody routed is a `405`, decided by the router:
//!
//! ```text
//! curl -si -X DELETE -H 'X-Principal: ada' localhost:3001/authors/a_1
//! ```
//!
//! Watch the server's terminal while running them. Every operation that
//! *ran* prints a `[hook]` line — including the ones that failed, which is
//! the point of an audit trail: a `422` and a `409` are recorded as
//! carefully as a `201`.
//!
//! The refused `DELETE` is the exception, and deliberately so: `before`
//! returned `Err`, the handler returned early, and `after` never ran. There
//! is no outcome to record because the operation never happened — nothing
//! reached storage. Auditing the *attempt* is the `before` hook's job, and
//! is why it receives the context too.
//!
//! Note also that each error document's `id` is the correlation id on the
//! matching `[hook]` line. That is the thread tying a client's "here is my
//! error id" to the log line explaining it.

mod blog;
mod hooks;
mod http;
mod store;

use ash_jsonapi::hook::Hooks;

use blog::{Author, Post};
use hooks::{RequireAuthForWrites, Trace};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // One validator per resource, built from the JSON Schema `resource!`
    // derived from the attribute list. Registered in `blog`'s `registry!`,
    // which is what lets a handler find the right one by resource name.
    let posts_validator = Post::validator()?;
    let authors_validator = Author::validator()?;

    // The hooks, in order. `before` runs them in sequence and the first
    // refusal stops the operation, so the authorization check is registered
    // ahead of the trace.
    //
    // `Audit::new(logger)` slots in exactly here — it is a `Hook` like these
    // two, writing a structured record per operation instead of a line.
    let hooks = Hooks::new().with(RequireAuthForWrites).with(Trace);

    let state = store::App::seeded(posts_validator, authors_validator, hooks);
    http::serve(state).await?;
    Ok(())
}
