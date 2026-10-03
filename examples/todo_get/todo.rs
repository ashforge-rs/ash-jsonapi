//! The JSON:API half: what a `Todo` *is* on the wire, and how it is fetched.
//!
//! Nothing in this file mentions axum, a router, a port or a request. It
//! names the resource, declares its wire format, and implements the two
//! read operations — which is the whole JSON:API surface of this service.
//! [`crate::http`] turns that into an HTTP server.
//!
//! The split is the one the crate itself draws: `resource!` and the
//! operation traits describe a resource, and `api!` is a separate macro that
//! routes one. Everything here would work unchanged behind a different
//! transport.

use ash_jsonapi::crud::{List, Listing, Read, Storage, Stream, Validators};
use ash_jsonapi::{Context, JsonApiError, Page, Query};

use crate::store::{App, StoreError};

/// One row. An ordinary struct — nothing here is derived from a framework
/// trait, and [`resource!`](ash_jsonapi::resource!) below is what teaches
/// the crate to serialize it.
#[derive(Clone)]
pub struct Todo {
    pub id: String,
    pub message: String,
    pub done: bool,
}

/// Where the collection is mounted. Named once here, because it is a fact
/// about the resource rather than about the server: `resource!` puts it in
/// every `self` link, and [`crate::http`] routes it.
pub const BASE_PATH: &str = "/todos";

// Declared once: the wire format, the request body type and its parsing, the
// serialization, and the JSON Schema bodies are validated against.
//
// `input: NewTodo` is generated even though nothing here accepts a body —
// the declaration describes the resource, and which operations you expose is
// `api!`'s business, over in `http`.
ash_jsonapi::resource! {
    Todo as "todo" at "/todos",
    state: App,
    schema: TodoSchema,
    input: NewTodo,
    id: id,
    attributes: { message: String, done: bool },
}

// The whole wiring, for a service that accepts no request bodies. A
// validator checks a body, and `GET` has none, so `Validators::validator`
// defaults to `None` and there is nothing to register.
//
// Route `CREATE` or `UPDATE` and this becomes an `ash_jsonapi::registry!`
// naming each resource's validator — a body-accepting operation with none
// registered fails as the wiring bug it is, rather than skipping validation.
impl Validators for App {}

// The error type every operation on this resource reports.
impl Storage<Todo> for App {
    type Error = StoreError;

    /// What `?sort=` may name.
    ///
    /// Declaring it means an unknown field is refused with a `400` before
    /// `list` runs — so the failure a client is most likely to cause cannot
    /// arrive as an opaque `500`, whatever `classify` below does or does not
    /// recognise.
    fn sortable(&self) -> Option<&[&str]> {
        Some(&["id", "message", "done"])
    }

    /// Which failures are the client's fault rather than the server's.
    ///
    /// Without this every error is an opaque `500`, which tells a client to
    /// retry a request that will never work: a `?sort=` field this service
    /// cannot order on is wrong *as sent*, and no amount of retrying fixes
    /// it. This is where those become a `400` naming the parameter at fault.
    ///
    /// Anything unrecognised falls through to the default `500`, and its
    /// text is logged rather than serialized — an error message can carry a
    /// connection string, so the opaque case has to stay the default.
    fn classify(&self, err: &Self::Error, ctx: &Context) -> Option<JsonApiError> {
        // A typed error rather than a string, so this match is exhaustive:
        // add a variant and the compiler asks how a client should see it.
        match err {
            StoreError::UnsupportedPaging => Some(ash_jsonapi::invalid_parameter!(
                ctx,
                "this service pages by `page[offset]`/`page[limit]`",
                "page"
            )),
        }
    }
}

impl Read<Todo> for App {
    /// `GET /todos/{id}`. `None` becomes a JSON:API `404` — the handler
    /// writes the error document, so there is nothing to build here.
    async fn get(&self, id: &str) -> Result<Option<Todo>, StoreError> {
        Ok(self.fetch(id))
    }
}

impl List<Todo> for App {
    /// `GET /todos`. The `?page[…]` and `?sort=` the client sent arrive
    /// parsed and bounds-checked; applying them is storage's job, because
    /// only storage knows what it can order on.
    async fn list(&self, query: &Query, _ctx: &Context) -> Result<Listing<Todo>, StoreError> {
        // This service pages by offset only. Refusing the other strategy is
        // better than quietly serving page one.
        let Page::Offset { offset, limit } = query.page() else {
            return Err(StoreError::UnsupportedPaging);
        };

        // `sortable` above already refused anything not in the list, so a key
        // that reaches here names a field this resource really has.
        let sort = query
            .sort()
            .first()
            .map(|key| (SortField::parse(key.field()), key.descending()));

        let (rows, total) = self.page(sort, *offset, *limit);

        // The total is what gives the response its `last` link and
        // `meta.total`; counting an in-memory map is free, so it is reported.
        Ok(Listing::new(rows).total(total))
    }
}

/// The attributes this resource can be ordered by.
///
/// A closed set rather than a string: `?sort=` naming anything else is the
/// client's mistake, and saying so is better than ignoring it.
#[derive(Clone, Copy)]
pub enum SortField {
    Message,
    Done,
    Id,
}

impl SortField {
    /// Infallible: `Storage::sortable` is the gate, and this list is the same
    /// one. A field that is not in it never reaches here.
    fn parse(field: &str) -> Self {
        match field {
            "message" => Self::Message,
            "done" => Self::Done,
            _ => Self::Id,
        }
    }
}

impl Stream<Todo> for App {
    /// `GET /todos` with `Accept: application/jsonl`.
    ///
    /// The whole collection, one row per line, with no page to hold it. Takes
    /// `self` by value because the stream outlives this call — it is polled
    /// while the body is written — and `App` is a cheap handle to clone.
    ///
    /// This one is backed by a `Vec`, so it is a stream over what is already
    /// in memory; a database-backed service hands back its driver's row
    /// stream instead and never materializes the collection at all.
    fn stream(
        self,
        query: Query,
        _ctx: Context,
    ) -> impl futures_core::Stream<Item = Result<Todo, StoreError>> + Send + Unpin + 'static {
        // Sorting still applies: a stream without an order is an arbitrary
        // sequence, exactly as a page without one is an arbitrary subset.
        let sort = query
            .sort()
            .first()
            .map(|key| (SortField::parse(key.field()), key.descending()));

        let (rows, _) = self.page(sort, 0, usize::MAX);
        futures_util::stream::iter(rows.into_iter().map(Ok))
    }
}
