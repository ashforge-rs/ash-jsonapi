//! Serve [JSON:API](https://jsonapi.org) from axum.
//!
//! Describe a resource once, and get the wire format for free: the
//! `type`/`id` envelope, relationship linkage, `self`/`related` links,
//! spec-shaped errors with JSON Pointers, and request-body validation.
//!
//! # One resource, end to end
//!
//! ```
//! use std::collections::HashMap;
//! use std::sync::{Arc, Mutex};
//!
//! use ash_jsonapi::Context;
//! use ash_jsonapi::crud::{Create, List, Listing, Read, Storage};
//! use ash_jsonapi::{Page, Query};
//! use ash_jsonapi::validation::DocumentValidator;
//!
//! #[derive(Clone)]
//! struct Todo { id: String, message: String, done: bool }
//!
//! // Your application state, however you already build it.
//! #[derive(Clone)]
//! struct App {
//!     rows: Arc<Mutex<HashMap<String, Todo>>>,
//!     validator: Arc<DocumentValidator>,
//! }
//!
//! // Declared once: this writes `TodoSchema`, the `NewTodo` request body and
//! // its parsing, the serialization, and the JSON Schema bodies are checked
//! // against. See `resource!` for what each line turns into.
//! ash_jsonapi::resource! {
//!     Todo as "todo" at "/todos",
//!     state: App,
//!     schema: TodoSchema,
//!     input: NewTodo,
//!     id: id,
//!     attributes: { message: String, done: bool },
//! }
//!
//! // Where the validators live. Needed here because this service routes
//! // CREATE, which has a body to check; a read-only service writes
//! // `impl Validators for App {}` instead and carries no validator at all.
//! ash_jsonapi::registry! {
//!     App { validator: Todo }
//! }
//!
//! // Your storage. One trait per operation, so you implement only what you
//! // route — this service creates and reads, so there is no `Update` or
//! // `Delete` impl anywhere.
//! impl Storage<Todo> for App {
//!     type Error = String;
//! }
//!
//! impl Create<Todo> for App {
//!     // The parsed body arrives as an argument.
//!     async fn create(&self, input: NewTodo, _ctx: &Context) -> Result<Todo, String> {
//!         let mut rows = self.rows.lock().unwrap();
//!         let todo = Todo {
//!             id: format!("t_{}", rows.len() + 1),
//!             message: input.message,
//!             done: input.done,
//!         };
//!         rows.insert(todo.id.clone(), todo.clone());
//!         Ok(todo)
//!     }
//! }
//!
//! impl Read<Todo> for App {
//!     async fn get(&self, id: &str) -> Result<Option<Todo>, String> {
//!         Ok(self.rows.lock().unwrap().get(id).cloned())
//!     }
//! }
//!
//! impl List<Todo> for App {
//!     // `?page[…]` and `?sort=` arrive parsed; applying them is yours.
//!     async fn list(&self, query: &Query, _ctx: &Context) -> Result<Listing<Todo>, String> {
//!         let rows = self.rows.lock().unwrap();
//!         let total = rows.len();
//!         let Page::Offset { offset, limit } = query.page() else {
//!             return Err("this service pages by offset".into());
//!         };
//!
//!         let page = rows.values().skip(*offset).take(*limit).cloned();
//!         Ok(Listing::new(page).total(total))
//!     }
//! }
//!
//! // The routes: `GET|POST /todos` and `GET /todos/{id}`, and nothing else.
//! ash_jsonapi::api! {
//!     operations: [ash_jsonapi::LIST, ash_jsonapi::CREATE, ash_jsonapi::READ],
//!     resource: Todo,
//!     name: "todo",
//!     path: "/todos",
//!     state: App,
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let state = App {
//!     rows: Arc::new(Mutex::new(HashMap::new())),
//!     validator: Arc::new(Todo::validator()?),
//! };
//!
//! // `layer()` attaches the per-request `Context` every handler needs.
//! // Without it each request is a 500, so it is not optional.
//! let app: axum::Router = routes()
//!     .layer(ash_jsonapi::layer())
//!     .with_state(state);
//!
//! // Then serve it:
//! //   let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//! //   axum::serve(listener, app).await?;
//! # Ok(())
//! # }
//! ```
//!
//! That is the whole service. `routes()` is an `axum::Router<App>`; name a
//! different set in `operations` to expose more or less, and only those
//! operations' traits are required — so a service backed by no storage at all
//! is `operations: [READ]` over whatever you compute on the fly.
//!
//! The two macros are independent. [`resource!`] writes the wire format and
//! [`api!`] the routes, and either can be replaced by the traits it stands
//! for — see [`crud`] and [`serialize`] for the hand-written form, which is
//! what you want for a computed attribute or a body that is not JSON:API.
//!
//! # Features
//!
//! | Feature | Default | What it adds |
//! |---|---|---|
//! | `http` | yes | The axum integration: [`api!`], [`response`] |
//! | `validation` | yes | [`validation::DocumentValidator`] — request bodies vs. a JSON Schema |
//! | `domain` | no | [`ash-domain`](crate::domain) integration |
//! | `audit` | yes | [`audit::Audit`] — one recorded event per operation |
//! | `streaming` | no | [`jsonl`] — JSONL in and out: the `STREAM` and `INGEST` operations |
//!
//! That is the whole list. TLS, health, metrics, OpenTelemetry, OpenAPI and
//! idempotency are planned rather than shipped, and are deliberately *not*
//! declared as features until they gate real code — a feature that compiles
//! and does nothing is worse than one that is absent.
//!
//! With `default-features = false` you get the document, error and
//! serialization types alone — no axum, no async runtime.
//!
//! # Using `ash-domain`?
//!
//! Optional, and off by default. Enable the `domain` feature and
//! [`domain_resource!`] writes the same traits from the schema the domain
//! already holds — no attribute, type or relationship is restated. Actions
//! run through the domain's own pipeline, so policies and extensions apply
//! and a denial reaches the client as a `403`. See [`crate::domain`].
//!
//! Everything above works with no `ash-*` crate in the tree; the integration
//! is a second way in, not the way in.
//!
//! # Layout
//!
//! - [`document`] — the JSON:API wire types.
//! - [`serialize`] — your data → [`ResourceObject`], driven by [`Schema`].
//! - [`error`] — spec-shaped errors and their statuses.
//! - [`validation`] — request-body checking.
//! - [`extract`] — [`FromRequest`](extract::FromRequest): a request → your types.
//! - [`layer`](mod@layer) — [`layer()`](fn@layer): the per-request [`Context`] middleware.
//! - [`attribute`] — [`Attribute`]: a field's type, schema and conversions,
//!   and [`Formatted`] for `format`-checked strings.
//! - [`declare`] — [`resource!`]: one declaration, all four impls.
//! - [`query`] — [`Query`]: `?page[…]` and `?sort=`, parsed and checked.
//! - [`crud`] — [`Resource`](crud::Resource), the per-operation traits, and [`api!`].
//! - files — [`Upload`](extract::Upload) in, [`Download`](response::Download)
//!   out, and the `CONTENT` operation that routes `/{id}/content`.
//! - [`jsonl`] — one resource per line, streamed, for collections too large
//!   to be a document. `STREAM` sends them, selected by `Accept`; `INGEST`
//!   reads them, selected by `Content-Type`, answering one result line per
//!   input line so a partial batch is visible rather than guessed at.
//! - [`response`] — axum responses.
//! - [`context`] — per-request correlation id and principal.
//! - [`macros`] — the one-liners a handler is made of: [`not_found!`],
//!   [`ensure!`], [`found!`], [`router!`], [`serve!`].
//! - [`i18n`] — [`Translation`] keys on errors, and the negotiated [`Locale`].
//! - [`hook`] — [`before`](hook::Hook::before)/[`after`](hook::Hook::after)
//!   around every operation; what audit, and later metrics, are built on.
//! - [`audit`] — [`Audit`](audit::Audit): the audit trail, as a hook.

//!
//! # Shorthand
//!
//! The macros in [`macros`] are sugar over what is above — nothing there is
//! unreachable by writing the long form. What they buy is that the short form
//! is also the correct one: an error keeps its correlation id, a router keeps
//! its [`layer()`](layer()), and a server keeps its shutdown signal.
//!
//! ```ignore
//! // A guard, a lookup, and the service — three lines that are hard to get
//! // subtly wrong:
//! ash_jsonapi::ensure!(!name.is_empty(), ctx, "name must not be empty", "name");
//! let todo = ash_jsonapi::found!(row, ctx, "no todo with that id");
//!
//! let app = ash_jsonapi::router!(App, todos::routes()).with_state(state);
//! ash_jsonapi::serve!("0.0.0.0:3000", app).await?;
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "domain")]
#[doc(hidden)]
pub use ash_domain as __ash_domain;

/// Re-exported so the macros can name it without the caller depending on it.
#[doc(hidden)]
pub use serde_json as __serde_json;

pub mod attribute;
#[cfg(feature = "audit")]
#[cfg_attr(docsrs, doc(cfg(feature = "audit")))]
pub mod audit;
pub mod context;
#[cfg(all(feature = "http", feature = "validation"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "http", feature = "validation"))))]
pub mod declare;
pub mod document;
#[cfg(feature = "domain")]
#[cfg_attr(docsrs, doc(cfg(feature = "domain")))]
pub mod domain;
pub mod error;
pub mod hook;
pub mod i18n;
#[cfg(all(feature = "streaming", feature = "http"))]
#[cfg_attr(docsrs, doc(cfg(feature = "streaming")))]
pub mod jsonl;
pub mod macros;
pub mod query;
#[cfg(feature = "validation")]
#[cfg_attr(docsrs, doc(cfg(feature = "validation")))]
pub mod validation;

#[cfg(all(feature = "http", feature = "validation"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "http", feature = "validation"))))]
pub mod crud;

#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
pub mod extract;

#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
pub mod layer;
#[cfg(feature = "http")]
#[cfg_attr(docsrs, doc(cfg(feature = "http")))]
pub mod response;
pub mod serialize;

pub use attribute::{Attribute, FormatName, Formatted};
pub use context::Context;
pub use document::{
    Document, ErrorObject, ErrorSource, Linkage, Links, MEDIA_TYPE, PrimaryData, Relationship,
    ResourceIdentifier, ResourceObject,
};
pub use error::{ErrorCode, JsonApiError};
pub use i18n::{Locale, Translation};
#[cfg(feature = "http")]
pub use layer::layer;
#[cfg(feature = "http")]
pub use macros::shutdown_signal;
pub use query::{Page, Paginator, Query, Sort};
pub use serialize::{NoRelationships, Rel, RelationshipDef, Schema, Serializer};
