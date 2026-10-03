//! Generated CRUD routes.
//!
//! The four handlers for a resource differ only in its type and its
//! relationships — everything else is the same shape every time. [`api!`](crate::api!)
//! writes them, against whatever storage you already have.
//!
//! Two traits connect them to your code:
//!
//! - [`Resource`] — how one of your types becomes a JSON:API resource
//!   object, and how a request document becomes one of your types.
//! - [`Validators`] — where the validators live, on your application state.
//! - [`Storage<R>`](Storage) — names the error type for one resource, plus
//!   one trait per operation: [`Create`], [`Read`], [`Update`], [`Delete`].
//!   Implement only the ones you route — a read-only proxy writes [`Read`]
//!   and nothing else.
//!
//! ```ignore
//! ash_jsonapi::api! {
//!     operations: [ash_jsonapi::CRUD],
//!     resource: Todo,
//!     name: "todo",
//!     path: "/api/v1/todos",
//!     state: App,
//! }
//! ```
//!
//! [`resource!`](crate::resource!) writes the [`Resource`] impl for you, so
//! in practice what you write here is the storage: one trait per operation
//! you route.
//!
//! The generated handlers take the request's [`Context`] as an axum
//! extension, so the router needs [`ash_jsonapi::layer()`](crate::layer())
//! — without it every request is a `500`.

use crate::context::Context;
use crate::document::ResourceObject;
use crate::error::JsonApiError;
use crate::extract::FromRequest;
use crate::query::Query;
use crate::response::Errors;
use crate::validation::DocumentValidator;

/// One of your types, as a JSON:API resource.
///
/// The two directions a handler needs: reading a validated request document
/// into your type, and writing your type back out as a resource object.
pub trait Resource<S: Validators>: Sized + Send + 'static {
    /// What a create body turns into.
    ///
    /// Its [`FromRequest`] impl owns reading and validating the request, so a
    /// resource whose body is not JSON:API — multipart, form-encoded — needs
    /// no special case here.
    ///
    /// Usually `Self`, but it can be a distinct type when a create carries
    /// different fields from a stored row.
    type Create: FromRequest<S>;

    /// What an update body turns into. Usually the same as
    /// [`Self::Create`](Resource::Create).
    type Update: FromRequest<S>;

    /// The JSON:API `type` member, e.g. `"todo"`.
    const NAME: &'static str;

    /// Where the resource is mounted, e.g. `/api/v1/todos`.
    const PATH: &'static str;

    /// Write this row out as a resource object, links and linkage included.
    ///
    /// `state` is your application state, for anything resolved at startup
    /// rather than compiled in — a schema registry, a base URL, a feature
    /// flag deciding whether an attribute is exposed. Ignore it when the
    /// resource knows its own shape, which is the usual case.
    fn to_resource(&self, state: &S, ctx: &Context) -> Result<ResourceObject, JsonApiError>;
}

/// Your application state, as the generated handlers see it.
///
/// Only the validators live here, because they are per-resource but built
/// once at startup. The operations are on [`Create`], [`Read`], [`Update`]
/// and [`Delete`] — one trait each, so a resource implements only what it
/// actually serves.
///
/// # A read-only service implements this for free
///
/// Validators check *request bodies*, and `GET` has none. A service that
/// routes only [`LIST`](crate::api!) and [`READ`](crate::api!) therefore
/// never reaches a validator, so the method below defaults to `None` and
/// such a service needs no [`registry!`](crate::registry!) at all:
///
/// ```ignore
/// // The whole implementation, for a service that accepts no bodies.
/// impl Validators for App {}
/// ```
///
/// Route an operation that *does* carry a body — `CREATE` or `UPDATE` —
/// without registering its validator, and the request fails as the wiring
/// bug it is, naming the resource. Use [`registry!`](crate::registry!) to
/// write the lookup rather than spelling it out.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not wired up as ash-jsonapi application state",
    label = "this state needs a `Validators` impl",
    note = "a service that accepts no request bodies needs only an empty impl: `impl ash_jsonapi::crud::Validators for {Self}` with no body",
    note = "one that routes CREATE or UPDATE registers its validators instead, with the `ash_jsonapi::registry!` macro"
)]
pub trait Validators: Clone + Send + Sync + 'static {
    /// The compiled request-body validator for one resource, by name.
    ///
    /// `None` means the resource has no validator registered. For a service
    /// that accepts no request bodies that is the correct answer and the
    /// default; for one that routes `CREATE` or `UPDATE` it is a wiring bug,
    /// and reported as one rather than as a `404`.
    fn validator(&self, _resource: &str) -> Option<&DocumentValidator> {
        None
    }

    /// The hooks to run around every operation on this state.
    ///
    /// `None` — the default — skips the mechanism entirely: no event is
    /// built and no hook list is walked, so a service that registers no
    /// hooks pays nothing for their existence.
    ///
    /// This lives on `Validators` rather than a trait of its own because every
    /// state already implements `Validators`; a second required trait would make
    /// every existing service add an empty impl to keep compiling, and a
    /// blanket impl would make the override impossible rather than optional.
    ///
    /// ```ignore
    /// impl Validators for App {
    ///     fn hooks(&self) -> Option<&Hooks> {
    ///         Some(&self.hooks)
    ///     }
    /// }
    /// ```
    fn hooks(&self) -> Option<&crate::hook::Hooks> {
        None
    }
}

/// What a resource's operations fail with.
///
/// Split from the operation traits so an implementation names its error once
/// rather than in each of the four.
///
/// The error reaches the client as a `500` carrying only the correlation id —
/// the `Display` output is logged, never serialized — so it is safe to return
/// whatever your data layer produces.
pub trait Storage<R>: Validators {
    /// Whatever your storage fails with.
    type Error: std::fmt::Display + Send;

    /// Report a failure that is the client's fault, not the server's.
    ///
    /// Storage errors are `500`s by default, and deliberately opaque: the
    /// `Display` output is logged, never serialized, so a connection string
    /// in an error message cannot reach a client.
    ///
    /// Some failures are not the server's fault, though — a `?sort=` field
    /// the table cannot order on, a cursor that has expired, a page beyond
    /// what the backend will serve. Returning `500` for those tells the
    /// client to retry something that will never work. Override this to
    /// classify them:
    ///
    /// ```ignore
    /// fn classify(&self, err: &Self::Error, ctx: &Context) -> Option<JsonApiError> {
    ///     match err {
    ///         Error::UnsortableField(field) => Some(
    ///             JsonApiError::in_request(ErrorCode::InvalidParameter, ctx)
    ///                 .detail(format!("cannot sort on `{field}`"))
    ///                 .parameter("sort"),
    ///         ),
    ///         // Everything else stays a logged 500.
    ///         _ => None,
    ///     }
    /// }
    /// ```
    ///
    /// `None` — the default — keeps whatever you return out of the response
    /// body. Whatever you *do* return is sent to the client, so return only
    /// what you would put in a log anyone can read. `ctx` is the request's,
    /// so the error carries the same correlation id as everything else it
    /// will be read beside.
    ///
    /// Note that the most common reason to reach for this — a `?sort=` field
    /// you cannot order by — is already handled: declare
    /// [`sortable`](Storage::sortable) and the request is refused with a
    /// `400` before `list` is ever called.
    fn classify(&self, err: &Self::Error, ctx: &Context) -> Option<JsonApiError> {
        let _ = (err, ctx);
        None
    }

    /// The attributes `?sort=` may name, or `None` for "anything goes".
    ///
    /// Declaring this closes the gap that
    /// [`classify`](Storage::classify) otherwise leaves open. A `?sort=`
    /// naming something absent from this list is the client's mistake, and
    /// without the list it reaches `list`, comes back as a storage error, and
    /// — unless `classify` recognises it — is answered with a `500` telling
    /// the client to retry a request that can never succeed.
    ///
    /// With the list, the same request is refused with a `400` naming the
    /// `sort` parameter, before storage is touched:
    ///
    /// ```ignore
    /// impl Storage<Todo> for App {
    ///     type Error = StoreError;
    ///
    ///     fn sortable(&self) -> Option<&[&str]> {
    ///         Some(&["id", "message", "done"])
    ///     }
    /// }
    /// ```
    ///
    /// `None` — the default — preserves the old behaviour for storage that
    /// can order by anything, or that would rather decide for itself.
    fn sortable(&self) -> Option<&[&str]> {
        None
    }
}

/// `POST /resource` — persist a new row.
///
/// Implement it only if you route [`CREATE`](crate::api!); a resource that
/// does not accept creates needs no impl at all.
///
/// ```ignore
/// impl Create<Todo> for App {
///     async fn create(&self, input: NewTodo, ctx: &Context) -> Result<Todo, Self::Error> {
///         sqlx::query_as("insert into todos …")
///             .bind(&input.message)
///             .fetch_one(&self.db)
///             .await
///     }
/// }
/// ```
pub trait Create<R: Resource<Self>>: Storage<R> {
    /// Persist a new row, assigning its id.
    ///
    /// The parsed body arrives as an argument, built by
    /// [`Resource::Create`]'s [`FromRequest`] impl. `ctx` comes alongside it
    /// for the principal, the tenant, and anything else bound to the
    /// request.
    fn create(
        &self,
        input: R::Create,
        ctx: &Context,
    ) -> impl Future<Output = Result<R, Self::Error>> + Send;
}

/// One page of a collection, on its way back to the client.
///
/// Storage returns the rows plus whatever it knows about the rest of the
/// collection; the handler turns that into `first`/`prev`/`next`/`last`
/// links. What is absent is simply not linked — a cursor-paged listing that
/// reports no total gets no `last` link rather than a wrong one.
pub struct Listing<R> {
    /// The rows on this page.
    pub rows: Vec<R>,
    /// How many rows the whole collection holds, if storage counted.
    ///
    /// Drives the `last` link and `meta.total`. Leave it `None` when
    /// counting costs a second query you do not want to pay for.
    pub total: Option<usize>,
    /// Where the next page resumes, for cursor paging.
    ///
    /// `None` means this is the last page, so no `next` link is emitted.
    /// Ignored under offset paging, which derives `next` from
    /// [`has_more`](Listing::has_more) or, failing that, arithmetically.
    pub cursor: Option<String>,
    /// Whether the collection continues past this page, when storage knows.
    ///
    /// Offset paging only, and an override for the `next` link: without it a
    /// full page is *assumed* to have a successor, so the last full page
    /// links to an empty one. Storage that can answer exactly — by reading
    /// one row past the page, say — sets this and the guess is not used.
    ///
    /// `None` means "not known", not "no more".
    pub has_more: Option<bool>,
}

impl<R> Listing<R> {
    /// A page of rows, with nothing else known about the collection.
    pub fn new(rows: impl IntoIterator<Item = R>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
            total: None,
            cursor: None,
            has_more: None,
        }
    }

    /// How many rows the collection holds in total.
    #[must_use]
    pub fn total(mut self, total: usize) -> Self {
        self.total = Some(total);
        self
    }

    /// Whether the collection continues past this page.
    ///
    /// Set it when storage knows exactly, and the `next` link stops being a
    /// guess — see [`has_more`](Listing::has_more).
    #[must_use]
    pub fn has_more(mut self, has_more: bool) -> Self {
        self.has_more = Some(has_more);
        self
    }

    /// The cursor the next page resumes from.
    ///
    /// Set it only when there *is* a next page: it is what emits the `next`
    /// link, so a cursor on the final page walks the client into an empty
    /// one.
    #[must_use]
    pub fn cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }
}

/// `GET /resource` — fetch a page of the collection.
///
/// Implement it only if you route [`LIST`](crate::api!).
///
/// ```ignore
/// impl List<Todo> for App {
///     async fn list(&self, query: &Query, ctx: &Context) -> Result<Listing<Todo>, Self::Error> {
///         match query.page() {
///             Page::Offset { offset, limit } => { /* … */ }
///             // Storage that pages one way refuses the other rather than
///             // pretending; `query.sort()` is likewise yours to apply.
///             Page::Cursor { .. } => { /* … */ }
///         }
///     }
/// }
/// ```
pub trait List<R: Resource<Self>>: Storage<R> {
    /// Fetch one page.
    ///
    /// The [`Query`] carries the paging the client asked for, already parsed
    /// and bounds-checked, and its sort keys. An empty collection is an empty
    /// [`Listing`], not a `404`.
    fn list(
        &self,
        query: &Query,
        ctx: &Context,
    ) -> impl Future<Output = Result<Listing<R>, Self::Error>> + Send;
}

/// `GET /resource/{id}` — fetch one row.
///
/// Implement it only if you route [`READ`](crate::api!).
pub trait Read<R: Resource<Self>>: Storage<R> {
    /// Fetch one row by id. `None` becomes a `404`.
    fn get(&self, id: &str) -> impl Future<Output = Result<Option<R>, Self::Error>> + Send;
}

/// `PATCH /resource/{id}` — replace a row.
///
/// Implement it only if you route [`UPDATE`](crate::api!).
pub trait Update<R: Resource<Self>>: Storage<R> {
    /// Replace the row at `id`.
    ///
    /// The parsed body arrives as an argument, as with [`Create::create`].
    /// The `id` is the path's, which is authoritative over anything the body
    /// carries.
    fn update(
        &self,
        id: &str,
        input: R::Update,
        ctx: &Context,
    ) -> impl Future<Output = Result<R, Self::Error>> + Send;
}

/// `GET /resource` with `Accept: application/jsonl` — rows, streamed.
///
/// Implement it only if you route [`STREAM`](crate::api!). It is the
/// streaming counterpart of [`List`]: where `list` returns a page that is
/// fully in memory before anything is sent, this yields rows as storage
/// produces them, and each is serialized and written before the next is
/// asked for.
///
/// That is the whole point. A `Vec<R>` of a million rows is a million rows in
/// memory; a stream of them is one. The same query that would exhaust the
/// heap as a `List` runs in constant space here.
///
/// ```ignore
/// impl Stream<Todo> for App {
///     fn stream(
///         self,
///         query: Query,
///         ctx: Context,
///     ) -> impl futures_core::Stream<Item = Result<Todo, Self::Error>> + Send + Unpin {
///         // A real implementation hands back its driver's row stream —
///         // `sqlx::query_as(…).fetch(&self.pool)` is already one.
///         futures_util::stream::iter(self.rows().into_iter().map(Ok))
///     }
/// }
/// ```
///
/// # Paging
///
/// The [`Query`] is passed for its sort keys and any limit the client set,
/// but a stream is not paged: the client asked for the whole collection and
/// gets it. Honour `page[limit]` as a cap if you want one — nothing in the
/// crate imposes it, because the point of streaming is to exceed what a page
/// can hold.
///
/// # Failing partway
///
/// Once the first row is written the status is `200`, so a later error
/// cannot change it. Yield `Err` and the stream ends with an error line —
/// see [`ERROR_LINE`](crate::jsonl::ERROR_LINE) for why a client must check
/// the last line rather than the status.
#[cfg(feature = "streaming")]
#[cfg_attr(docsrs, doc(cfg(feature = "streaming")))]
pub trait Stream<R: Resource<Self>>: Storage<R> {
    /// Yield the collection's rows, in order.
    ///
    /// Takes `self` and the query **by value**: the returned stream outlives
    /// this call — it is polled while the response body is written — so it
    /// cannot borrow either. Your state is [`Clone`] already (a cheap handle
    /// to a pool, in practice), which is what makes that free.
    fn stream(
        self,
        query: Query,
        ctx: Context,
    ) -> impl futures_core::Stream<Item = Result<R, Self::Error>> + Send + Unpin + 'static;
}

/// `POST /resource` with `Content-Type: application/jsonl` — a batch, streamed.
///
/// Implement it only if you route [`INGEST`](crate::api!). It is the
/// streaming counterpart of [`Create`]: where `create` reads one document and
/// answers with one resource, this reads a line at a time and answers with
/// one result line per input line.
///
/// There is a blanket impl over [`Create`], so a resource that already
/// creates gets ingest for free — routing `INGEST` is usually the only change:
///
/// ```ignore
/// ash_jsonapi::api! {
///     operations: [ash_jsonapi::CREATE, ash_jsonapi::INGEST],
///     resource: Todo, name: "todo", path: "/todos", state: App,
/// }
/// ```
///
/// Override it only when a batch write differs from a single one — to take a
/// connection once for the run, say, rather than per row.
///
/// # Partial success is the model
///
/// A line that fails does not stop the batch: it becomes an error line
/// carrying its line number, and the next line is attempted. That is the
/// whole difference from a transactional bulk endpoint, and it is deliberate
/// — an export of a million rows that is refused in full because row 999,999
/// was malformed is not a useful endpoint.
///
/// Nothing is rolled back, so a client reconciles from the result lines
/// rather than assuming all-or-nothing. See
/// [`LINE_MEMBER`](crate::jsonl::LINE_MEMBER).
#[cfg(feature = "streaming")]
#[cfg_attr(docsrs, doc(cfg(feature = "streaming")))]
pub trait Ingest<R: Resource<Self>>: Storage<R> {
    /// Persist one row of the batch.
    ///
    /// Called once per non-blank input line, in order, each awaited before
    /// the next line is read — so a slow store applies backpressure to the
    /// upload rather than buffering it.
    fn create_one(
        &self,
        input: R::Create,
        ctx: &Context,
    ) -> impl Future<Output = Result<R, Self::Error>> + Send;
}

#[cfg(feature = "streaming")]
impl<S, R> Ingest<R> for S
where
    S: Create<R>,
    R: Resource<S>,
{
    /// A batch row is an ordinary create unless storage says otherwise.
    async fn create_one(&self, input: R::Create, ctx: &Context) -> Result<R, Self::Error> {
        self.create(input, ctx).await
    }
}

/// `GET /resource/{id}/content` — the resource's bytes.
///
/// Implement it only if you route [`CONTENT`](crate::api!).
///
/// The seam for a resource that *is* a file: an avatar, an attachment, an
/// exported report. The row at `/docs/{id}` stays an ordinary JSON:API
/// document describing the file — its name, size and media type — and the
/// bytes live at `/docs/{id}/content`, because a JSON:API document has no
/// place to put them that is not base64 in an attribute.
///
/// ```ignore
/// impl Content<Doc> for App {
///     async fn content(&self, id: &str) -> Result<Option<Download>, Self::Error> {
///         let Some(doc) = self.fetch(id) else { return Ok(None) };
///         Ok(Some(
///             Download::new(doc.bytes, doc.content_type).filename(doc.name),
///         ))
///     }
/// }
/// ```
///
/// `None` is a `404`, exactly as [`Read::get`] — a row whose bytes are
/// missing is not found, not a server error.
pub trait Content<R: Resource<Self>>: Storage<R> {
    /// Fetch one resource's bytes.
    ///
    /// Returns a [`Download`](crate::response::Download), which carries the
    /// media type and filename and sets the headers that keep a browser from
    /// interpreting the bytes.
    fn content(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<Option<crate::response::Download>, Self::Error>> + Send;
}

/// `DELETE /resource/{id}` — remove a row.
///
/// Implement it only if you route [`DELETE`](crate::api!).
pub trait Delete<R: Resource<Self>>: Storage<R> {
    /// Remove the row at `id`.
    fn destroy(&self, id: &str) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// The validator for one resource, or a logged `500`.
///
/// A resource whose validator is missing was never registered, which is a
/// wiring bug rather than a bad request.
pub fn validator<'a, S: Validators>(
    state: &'a S,
    resource: &str,
    ctx: &Context,
) -> Result<&'a DocumentValidator, Errors> {
    state
        .validator(resource)
        .ok_or_else(|| logged(ctx, format_args!("resource `{resource}` is not registered")))
}

/// Run a `before` hook set, if the state carries any.
///
/// Returns the refusal as [`Errors`] so a handler can `?` it: a refused
/// request never reaches storage, which is the point of `before`.
pub fn before<S: Validators>(
    state: &S,
    event: &crate::hook::Event<'_>,
    ctx: &Context,
) -> Result<(), Errors> {
    let Some(hooks) = state.hooks() else {
        return Ok(());
    };
    hooks.before(event, ctx).map_err(Errors::from)
}

/// Run the `after` hooks against whatever the operation produced.
///
/// Takes and returns the result unchanged: an `after` hook observes, and
/// threading the value through keeps that impossible to get wrong at a call
/// site. The outcome's status is the one the client will actually see.
pub fn after<S: Validators, T>(
    state: &S,
    event: &crate::hook::Event<'_>,
    ctx: &Context,
    result: Result<T, Errors>,
) -> Result<T, Errors> {
    let Some(hooks) = state.hooks() else {
        return result;
    };

    let outcome = match &result {
        // The success status is the operation's, not a guess: a create is a
        // `201` with a Location header and a delete a `204` with no body.
        Ok(_) => crate::hook::Outcome::Succeeded {
            status: match event.operation {
                crate::hook::Operation::Create => 201,
                crate::hook::Operation::Delete => 204,
                _ => 200,
            },
        },
        Err(errors) => crate::hook::Outcome::Failed {
            status: errors
                .0
                .iter()
                .map(|error| error.status().as_u16())
                .max()
                .unwrap_or(500),
        },
    };

    hooks.after(event, &outcome, ctx);
    result
}

/// Refuse a `?sort=` naming a field this storage did not declare.
///
/// Runs before the operation, so an unsortable field never reaches storage
/// and never has the chance to become an opaque `500`. Storage that declares
/// no list is trusted to sort by anything, and this is a no-op.
pub fn check_sort<S, R>(state: &S, query: &Query, ctx: &Context) -> Result<(), Errors>
where
    S: Storage<R>,
{
    let Some(sortable) = state.sortable() else {
        return Ok(());
    };

    for key in query.sort() {
        if !sortable.contains(&key.field()) {
            return Err(
                JsonApiError::in_request(crate::ErrorCode::InvalidParameter, ctx)
                    .detail(format!(
                        "cannot sort on `{}`; this collection sorts by {}",
                        key.field(),
                        sortable
                            .iter()
                            .map(|field| format!("`{field}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .parameter("sort")
                    .into(),
            );
        }
    }

    Ok(())
}

/// Drive a [`Stream`] into a JSONL body.
///
/// Each row is converted, serialized and yielded as it arrives — nothing
/// accumulates, which is the whole reason this exists rather than collecting
/// into a [`Listing`].
///
/// A row that fails, or that cannot be serialized, ends the stream with an
/// error line rather than truncating it: the status is already `200`, so a
/// client that sees a short stream and no error line would keep a partial
/// export believing it complete.
#[cfg(feature = "streaming")]
pub fn stream_lines<S, R>(state: S, query: Query, ctx: Context) -> crate::response::Streamed
where
    S: Stream<R> + Clone + Send + Sync + Unpin + 'static,
    R: Resource<S> + Send + 'static,
{
    use std::pin::Pin;
    use std::task::{Context as TaskContext, Poll};

    // A hand-written stream rather than a combinator chain: the rows borrow
    // `state` and `query`, so both have to live inside the stream itself.
    struct Lines<S, R, F> {
        rows: F,
        state: S,
        ctx: Context,
        done: bool,
        _row: std::marker::PhantomData<fn() -> R>,
    }

    impl<S, R, F> futures_core::Stream for Lines<S, R, F>
    where
        // `Unpin` so the stream can be polled through a `&mut`: the state is
        // only read here, never moved, and every real state satisfies it.
        S: Stream<R> + Send + Sync + Unpin + 'static,
        R: Resource<S> + Send + 'static,
        F: futures_core::Stream<Item = Result<R, S::Error>> + Send + Unpin,
    {
        type Item = Result<Vec<u8>, std::convert::Infallible>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
            let this = self.get_mut();
            if this.done {
                return Poll::Ready(None);
            }

            match Pin::new(&mut this.rows).poll_next(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(None) => {
                    this.done = true;
                    Poll::Ready(None)
                }
                Poll::Ready(Some(Ok(row))) => match row.to_resource(&this.state, &this.ctx) {
                    Ok(resource) => {
                        Poll::Ready(Some(Ok(crate::response::Streamed::line(&resource))))
                    }
                    Err(error) => {
                        this.done = true;
                        Poll::Ready(Some(Ok(crate::response::Streamed::error_line(
                            &error.in_context(&this.ctx),
                        ))))
                    }
                },
                Poll::Ready(Some(Err(err))) => {
                    this.done = true;
                    // `classify` decides what the client may know, exactly as
                    // it does for a non-streamed failure.
                    let error = this
                        .state
                        .classify(&err, &this.ctx)
                        .unwrap_or_else(|| JsonApiError::internal(&this.ctx));
                    Poll::Ready(Some(Ok(crate::response::Streamed::error_line(&error))))
                }
            }
        }
    }

    let rows = state.clone().stream(query, ctx.clone());

    crate::response::Streamed::new(Lines::<S, R, _> {
        rows,
        state,
        ctx,
        done: false,
        _row: std::marker::PhantomData,
    })
}

/// Drive a JSONL request body into a JSONL response body.
///
/// One result line per input line, in order: `{"data":{…}}` for a row that
/// landed, `{"errors":[…]}` carrying `meta.line` for one that did not. Each
/// line is read, parsed, validated, written and dropped before the next is
/// read, so a batch of any size costs a line of memory in each direction.
///
/// A failing line does not end the run — see [`Ingest`] for why partial
/// success is the model here rather than a transaction.
#[cfg(feature = "streaming")]
pub fn ingest_lines<S, R>(
    state: S,
    lines: crate::extract::JsonlLines,
    ctx: Context,
) -> crate::response::Streamed
where
    S: Ingest<R> + Clone + Send + Sync + Unpin + 'static,
    R: Resource<S> + Send + 'static,
{
    use std::pin::Pin;
    use std::task::{Context as TaskContext, Poll};

    /// What the driver is waiting on.
    ///
    /// A create is `async`, so the driver cannot simply read a line and
    /// return — it has to hold the in-flight future across polls. That is
    /// the whole reason this is a state machine rather than a `map` over the
    /// line stream.
    enum State<R> {
        /// Reading the next line from the body.
        Reading,
        /// A create is in flight for the line being held.
        ///
        /// The error is already-shaped JSON:API errors rather than the
        /// storage error: `Storage::Error` is only `Display + Send` with no
        /// `'static`, so it cannot cross into a boxed future. Classifying
        /// inside the future — where the state is owned — is what keeps a
        /// `422` a `422` instead of collapsing every failure to a `500`.
        Writing(
            usize,
            Pin<Box<dyn Future<Output = Result<R, Vec<JsonApiError>>> + Send>>,
        ),
    }

    struct Ingesting<S, R> {
        lines: crate::extract::JsonlLines,
        state: S,
        ctx: Context,
        phase: State<R>,
        done: bool,
    }

    impl<S, R> futures_core::Stream for Ingesting<S, R>
    where
        S: Ingest<R> + Clone + Send + Sync + Unpin + 'static,
        R: Resource<S> + Send + 'static,
    {
        type Item = Result<Vec<u8>, std::convert::Infallible>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
            let this = self.get_mut();

            loop {
                if this.done {
                    return Poll::Ready(None);
                }

                match &mut this.phase {
                    State::Writing(number, writing) => {
                        let number = *number;
                        let finished = std::task::ready!(writing.as_mut().poll(cx));
                        this.phase = State::Reading;

                        let line = match finished {
                            Ok(row) => match row.to_resource(&this.state, &this.ctx) {
                                Ok(resource) => crate::response::Streamed::result_line(&resource),
                                Err(error) => crate::response::Streamed::error_line_at(
                                    &error.in_context(&this.ctx),
                                    number,
                                ),
                            },
                            // The line did not land. The errors arrive
                            // already shaped — a validation failure keeps its
                            // pointer and its 422, a storage failure has been
                            // through `classify` — so nothing is re-decided
                            // here.
                            Err(errors) => {
                                let error = errors
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| JsonApiError::internal(&this.ctx));
                                crate::response::Streamed::error_line_at(&error, number)
                            }
                        };

                        return Poll::Ready(Some(Ok(line)));
                    }

                    State::Reading => {
                        let next = std::task::ready!(Pin::new(&mut this.lines).poll_next(cx));

                        let (number, line) = match next {
                            Some(item) => item,
                            None => {
                                this.done = true;
                                return Poll::Ready(None);
                            }
                        };

                        // A line that could not be read at all — over the
                        // cap, or a truncated body. `JsonlLines` has already
                        // ended itself, so this is the last line.
                        let bytes = match line {
                            Ok(bytes) => bytes,
                            Err(error) => {
                                this.done = true;
                                return Poll::Ready(Some(Ok(
                                    crate::response::Streamed::error_line_at(&error, number),
                                )));
                            }
                        };

                        // Each line is its own request document, so it is
                        // parsed and validated exactly as a single create's
                        // body would be — same pointers, same i18n keys.
                        let request = match axum::http::Request::builder()
                            .method(axum::http::Method::POST)
                            .header(
                                axum::http::header::CONTENT_TYPE,
                                crate::document::MEDIA_TYPE,
                            )
                            .body(axum::body::Body::from(bytes))
                        {
                            Ok(request) => request,
                            Err(_) => {
                                return Poll::Ready(Some(Ok(
                                    crate::response::Streamed::error_line_at(
                                        &JsonApiError::internal(&this.ctx),
                                        number,
                                    ),
                                )));
                            }
                        };

                        let state = this.state.clone();
                        let ctx = this.ctx.clone();

                        this.phase = State::Writing(
                            number,
                            Box::pin(async move {
                                type Input<S, R> = <R as Resource<S>>::Create;

                                // Parsing and validation, exactly as a single
                                // create does it. A failure here is the
                                // client's and keeps its own status.
                                let input = <Input<S, R> as FromRequest<S>>::from_request(
                                    request, &state, &ctx,
                                )
                                .await
                                .map_err(|errors| errors.0)?;

                                state.create_one(input, &ctx).await.map_err(|err| {
                                    // `classify` gets first refusal, as it
                                    // does for a non-streamed create; only
                                    // what it declines becomes a logged 500,
                                    // since the storage message must not
                                    // reach the client.
                                    match state.classify(&err, &ctx) {
                                        Some(error) => vec![error.in_context(&ctx)],
                                        None => {
                                            eprintln!(
                                                "[{}] ingest failed: {err}",
                                                ctx.correlation_id()
                                            );
                                            vec![JsonApiError::internal(&ctx)]
                                        }
                                    }
                                })
                            }),
                        );
                    }
                }
            }
        }
    }

    crate::response::Streamed::new(Ingesting::<S, R> {
        lines,
        state,
        ctx,
        phase: State::<R>::Reading,
        done: false,
    })
}

/// A storage error on its way to the client.
///
/// [`Storage::classify`] gets first refusal: a failure it names becomes that
/// error, correlation id attached. Anything else is logged with the id and
/// answered with a bare `500`.
pub fn failed<S, R>(state: &S, ctx: &Context, stage: &str, err: &S::Error) -> Errors
where
    S: Storage<R>,
{
    match state.classify(err, ctx) {
        Some(error) => error.in_context(ctx).into(),
        None => logged(ctx, format_args!("{stage} failed: {err}")),
    }
}

/// Log a cause, return a `500` that carries only the correlation id.
///
/// The single place the two halves are tied together: whatever goes to the
/// operator is prefixed with the id the client is handed.
fn logged(ctx: &Context, cause: std::fmt::Arguments<'_>) -> Errors {
    eprintln!("[{}] {cause}", ctx.correlation_id());
    JsonApiError::internal(ctx).into()
}

/// `404` for a row that does not exist.
pub fn missing(ctx: &Context, resource: &str, id: &str) -> Errors {
    JsonApiError::in_request(crate::ErrorCode::NotFound, ctx)
        .detail(format!("no {resource} with id `{id}`"))
        .into()
}

/// Generate JSON:API routes for one resource.
///
/// Pick the operations you want; nothing else is exposed. Routing every
/// action a resource declares just because it exists is how internal
/// operations leak, so this is opt-in.
///
/// ```ignore
/// ash_jsonapi::api! {
///     operations: [ash_jsonapi::CREATE, ash_jsonapi::READ],
///     resource: Todo,
///     name: "todo",
///     path: "/api/v1/todos",
///     state: App,
/// }
/// ```
///
/// | Operation | Route |
/// |---|---|
/// | `LIST` | `GET /todos` |
/// | `CREATE` | `POST /todos` |
/// | `READ` | `GET /todos/{id}` |
/// | `UPDATE` | `PATCH /todos/{id}` |
/// | `DELETE` | `DELETE /todos/{id}` |
/// | `CRUD` | all five |
/// | `STREAM` | `GET /todos`, when `Accept: application/jsonl` |
/// | `INGEST` | `POST /todos`, when `Content-Type: application/jsonl` |
/// | `CONTENT` | `GET /todos/{id}/content` |
///
/// `STREAM` and `INGEST` share a verb with `LIST` and `CREATE` rather than
/// adding a route: one URL with two representations, selected by the header.
/// Routing either alongside its document counterpart is the usual case and
/// changes nothing for an existing client. Routing `INGEST` *without*
/// `CREATE` makes the collection batch-only, and a document body there is a
/// `415`.
///
/// The names are macro keywords, not items, so they work bare (`READ`) or
/// qualified (`ash_jsonapi::READ`) — write whichever reads better at the call
/// site. An unrecognised name is a compile error naming the valid ones.
///
/// Anything not listed is not routed: a `[READ]` resource answers `405` to a
/// `DELETE`, rather than reaching a handler that then refuses.
///
/// Fields:
///
/// - `operations` — which routes to expose. Required: an empty list is a
///   resource with no HTTP surface, which is a mistake worth spelling out.
/// - `resource` — your resource type.
/// - `name` — its JSON:API `type`, for validator lookup.
/// - `path` — where it is mounted.
/// - `state` — your state type, implementing [`Validators`] and the traits for
///   each operation you list.
///
/// Expands to `pub fn routes() -> axum::Router<App>` plus the handlers, in a
/// private module. Write your own route for anything the generated set does
/// not cover; this is a starting point, not a ceiling.
#[macro_export]
macro_rules! api {
    (
        operations: [$($op:tt)+],
        resource: $resource:ident,
        name: $name:literal,
        path: $base:literal,
        state: $state:ident
        $(,)?
    ) => {
        /// Routes generated by `api!`.
        pub fn routes() -> ::axum::Router<$state> {
            // One `MethodRouter` per path, each selected operation adding its
            // verb. Building them this way means READ + UPDATE + DELETE share
            // the `/{id}` entry rather than clobbering one another.
            #[allow(unused_mut)]
            let mut collection = ::axum::routing::MethodRouter::<$state>::new();
            #[allow(unused_mut)]
            let mut member = ::axum::routing::MethodRouter::<$state>::new();
            // `CONTENT` is the one operation that is not a verb on an
            // existing path: bytes live beside the document, not instead of
            // it, so `/{id}/content` is its own entry.
            #[allow(unused_mut)]
            let mut content = ::axum::routing::MethodRouter::<$state>::new();

            $crate::__api_routes!(collection, member, content, __api, $($op)+);

            // The collection's `GET`, registered exactly once — STREAM and
            // LIST both want it, and two `.get()` calls on one MethodRouter
            // panic. Which handler answers is decided during expansion, so
            // only the one that exists is ever named.
            $crate::__api_collection_get!(collection, __api, [$($op)+], [$($op)+]);

            // The collection's `POST`, for the same reason: CREATE and INGEST
            // both want it, and only one handler may be registered.
            $crate::__api_collection_post!(collection, __api, [$($op)+], [$($op)+]);

            let router = ::axum::Router::new()
                .route($base, collection)
                .route(::std::concat!($base, "/{id}"), member);

            // Added only when CONTENT was routed: an empty MethodRouter
            // answers 405 on a path that should not exist at all.
            if $crate::__api_has_content!($($op)+) {
                router.route(::std::concat!($base, "/{id}/content"), content)
            } else {
                router
            }
        }

        #[doc(hidden)]
        mod __api {
            use super::*;

            $crate::__api_handlers!($resource, $name, $base, $state, $($op)+);

            // The `POST` dispatcher, when INGEST was routed. Emitted here
            // rather than from `__api_handlers!` because which fallback it
            // needs depends on the whole operation list, and that macro sees
            // one operation at a time.
            $crate::__api_maybe_ingest_dispatch!(
                $resource, $name, $base, $state, [$($op)+], [$($op)+]
            );

            use $crate::context::Context;
            #[allow(unused_imports)]
            use $crate::crud::{
                Content as _, Create as _, Delete as _, Read as _, Resource as _, Update as _,
                failed, missing,
            };
            use $crate::extract::FromRequest;
            use $crate::response::{Created, Errors, Fetched, NoContent};




        }

    };
}

/// Walks the operation list, tolerating `READ` and `ash_jsonapi::READ` alike.
///
/// A `tt` list rather than `path` fragments: a path cannot be matched against
/// literal names, and the leading segments are noise here — only the final
/// name selects a route.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_routes {
    // Done.
    ($collection:ident, $member:ident, $content:ident, $m:ident $(,)?) => {};

    // Skip a qualifying prefix: `ash_jsonapi :: READ` → `READ`.
    ($collection:ident, $member:ident, $content:ident, $m:ident, $prefix:ident :: $($rest:tt)+) => {
        $crate::__api_routes!($collection, $member, $content, $m, $($rest)+);
    };

    // One operation, then whatever follows.
    ($collection:ident, $member:ident, $content:ident, $m:ident, $op:ident $(, $($rest:tt)*)?) => {
        $crate::__api_route!($collection, $member, $content, $m, $op);
        $($crate::__api_routes!($collection, $member, $content, $m, $($rest)*);)?
    };
}

/// Whether the operation list names `CONTENT`, so `routes()` knows whether to
/// mount `/{id}/content` at all.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_has_content {
    ($($op:tt)*) => { $crate::__api_has!(CONTENT, $($op)*) };
}

/// Register the collection's `GET`, choosing the handler at expansion time.
///
/// Walks the operation list twice: the first copy looks for `STREAM` (which
/// wins, since its handler falls back to the document), the second for
/// `LIST`/`CRUD`. Naming a handler that was not generated would not compile,
/// so the choice cannot be a runtime `if`.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_collection_get {
    // STREAM found: the branching handler answers.
    ($collection:ident, $m:ident, [STREAM $(, $($a:tt)*)?], [$($b:tt)*]) => {
        $collection = $collection.get($m::list_or_stream);
    };
    ($collection:ident, $m:ident, [$p:ident :: $($a:tt)+], [$($b:tt)*]) => {
        $crate::__api_collection_get!($collection, $m, [$($a)+], [$($b)*]);
    };
    ($collection:ident, $m:ident, [$op:ident $(, $($a:tt)*)?], [$($b:tt)*]) => {
        $crate::__api_collection_get!($collection, $m, [$($($a)*)?], [$($b)*]);
    };
    // No STREAM anywhere: fall through to the LIST/CRUD pass.
    ($collection:ident, $m:ident, [], [$($b:tt)*]) => {
        $crate::__api_plain_list_get!($collection, $m, $($b)*);
    };
}

/// The LIST/CRUD half of [`__api_collection_get`].
#[doc(hidden)]
#[macro_export]
macro_rules! __api_plain_list_get {
    ($collection:ident, $m:ident, LIST $(, $($rest:tt)*)?) => {
        $collection = $collection.get($m::list);
    };
    ($collection:ident, $m:ident, CRUD $(, $($rest:tt)*)?) => {
        $collection = $collection.get($m::list);
    };
    ($collection:ident, $m:ident, $p:ident :: $($rest:tt)+) => {
        $crate::__api_plain_list_get!($collection, $m, $($rest)+);
    };
    ($collection:ident, $m:ident, $op:ident $(, $($rest:tt)*)?) => {
        $crate::__api_plain_list_get!($collection, $m, $($($rest)*)?);
    };
    // Neither LIST nor CRUD: the collection has no GET.
    ($collection:ident, $m:ident,) => {};
}

/// Whether the operation list names `$what`, ignoring any path prefix.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_has {
    ($what:ident,) => { false };
    ($what:ident, $prefix:ident :: $($rest:tt)+) => { $crate::__api_has!($what, $($rest)+) };
    ($what:ident, $op:ident $(, $($rest:tt)*)?) => {
        ::std::stringify!($op) == ::std::stringify!($what)
            || $crate::__api_has!($what, $($($rest)*)?)
    };
}

/// Attaches one operation's verb to the right `MethodRouter`.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_route {
    // LIST's `GET` is registered by `routes()` itself, because STREAM
    // replaces the handler on the same verb and two `.get()` calls on one
    // MethodRouter panic at startup.
    ($collection:ident, $member:ident, $content:ident, $m:ident, LIST) => {};
    // CREATE's `POST` is registered by `routes()` itself, for the same reason
    // LIST's `GET` is: INGEST replaces the handler on the same verb, and two
    // `.post()` calls on one MethodRouter panic at startup.
    ($collection:ident, $member:ident, $content:ident, $m:ident, CREATE) => {};
    ($collection:ident, $member:ident, $content:ident, $m:ident, READ) => {
        $member = $member.get($m::show);
    };
    ($collection:ident, $member:ident, $content:ident, $m:ident, UPDATE) => {
        $member = $member.patch($m::update);
    };
    ($collection:ident, $member:ident, $content:ident, $m:ident, DELETE) => {
        $member = $member.delete($m::destroy);
    };
    ($collection:ident, $member:ident, $content:ident, $m:ident, CONTENT) => {
        $content = $content.get($m::content);
    };
    // Likewise registered by `routes()`: STREAM selects which handler
    // answers the collection's `GET`, it does not add a second one.
    ($collection:ident, $member:ident, $content:ident, $m:ident, STREAM) => {};
    // Likewise registered by `routes()`: INGEST selects which handler answers
    // the collection's `POST`, it does not add a second one.
    ($collection:ident, $member:ident, $content:ident, $m:ident, INGEST) => {};
    ($collection:ident, $member:ident, $content:ident, $m:ident, CRUD) => {
        // The `POST` is left to `routes()` along with CREATE's, so that
        // `CRUD, INGEST` resolves to one handler rather than panicking.
        $member = $member.get($m::show).patch($m::update).delete($m::destroy);
    };
    ($collection:ident, $member:ident, $content:ident, $m:ident, $other:ident) => {
        ::std::compile_error!(::std::concat!(
            "unknown operation `",
            ::std::stringify!($other),
            "`; expected LIST, CREATE, READ, UPDATE, DELETE, CONTENT, STREAM, INGEST or CRUD"
        ));
    };
}

/// Register the collection's `POST`, choosing the handler at expansion time.
///
/// The `POST` counterpart of [`__api_collection_get`], and it exists for the
/// same reason: `CREATE` and `INGEST` both want the verb, and two `.post()`
/// calls on one `MethodRouter` panic at startup. `INGEST` wins where both are
/// named, because its handler falls back to the single create for any body
/// that is not JSONL.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_collection_post {
    // INGEST found: the branching handler answers.
    ($collection:ident, $m:ident, [INGEST $(, $($a:tt)*)?], [$($b:tt)*]) => {
        $collection = $collection.post($m::create_or_ingest);
    };
    ($collection:ident, $m:ident, [$p:ident :: $($a:tt)+], [$($b:tt)*]) => {
        $crate::__api_collection_post!($collection, $m, [$($a)+], [$($b)*]);
    };
    ($collection:ident, $m:ident, [$op:ident $(, $($a:tt)*)?], [$($b:tt)*]) => {
        $crate::__api_collection_post!($collection, $m, [$($($a)*)?], [$($b)*]);
    };
    // No INGEST anywhere: fall through to the CREATE/CRUD pass.
    ($collection:ident, $m:ident, [], [$($b:tt)*]) => {
        $crate::__api_plain_create_post!($collection, $m, $($b)*);
    };
}

/// Emit the `POST` dispatcher only when `INGEST` was actually routed.
///
/// The guard in front of [`__api_ingest_dispatch`]: a resource that routes no
/// batch must not get a `create_or_ingest`, since nothing would name it and
/// the `ingest` handler it calls was never generated.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_maybe_ingest_dispatch {
    // INGEST found: emit the dispatcher, choosing its fallback from the
    // second (untouched) copy of the operation list.
    ($r:ident, $name:literal, $base:literal, $state:ident, [INGEST $(, $($a:tt)*)?], [$($b:tt)*]) => {
        $crate::__api_ingest_dispatch!($r, $name, $base, $state, [$($b)*], [$($b)*]);
    };
    ($r:ident, $name:literal, $base:literal, $state:ident, [$p:ident :: $($a:tt)+], [$($b:tt)*]) => {
        $crate::__api_maybe_ingest_dispatch!($r, $name, $base, $state, [$($a)+], [$($b)*]);
    };
    ($r:ident, $name:literal, $base:literal, $state:ident, [$op:ident $(, $($a:tt)*)?], [$($b:tt)*]) => {
        $crate::__api_maybe_ingest_dispatch!($r, $name, $base, $state, [$($($a)*)?], [$($b)*]);
    };
    // No INGEST: nothing to emit.
    ($r:ident, $name:literal, $base:literal, $state:ident, [], [$($b:tt)*]) => {};
}

/// Emit the `POST` dispatcher, when `INGEST` was routed.
///
/// Two variants, chosen at expansion time, because the fallback has to name
/// a handler that exists: with `CREATE` (or `CRUD`) also routed a non-JSONL
/// body is an ordinary single create, and with `INGEST` alone there is no
/// `create` to call, so anything but JSONL is a `415`.
///
/// Walks the list twice like [`__api_collection_get`]: the first copy looks
/// for `CREATE`/`CRUD`, the second is kept whole so the search can restart.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_ingest_dispatch {
    // A single create exists: fall back to it.
    ($r:ident, $name:literal, $base:literal, $state:ident, [CREATE $(, $($a:tt)*)?], [$($b:tt)*]) => {
        $crate::__api_dispatch_with_create!($r, $name, $base, $state);
    };
    ($r:ident, $name:literal, $base:literal, $state:ident, [CRUD $(, $($a:tt)*)?], [$($b:tt)*]) => {
        $crate::__api_dispatch_with_create!($r, $name, $base, $state);
    };
    ($r:ident, $name:literal, $base:literal, $state:ident, [$p:ident :: $($a:tt)+], [$($b:tt)*]) => {
        $crate::__api_ingest_dispatch!($r, $name, $base, $state, [$($a)+], [$($b)*]);
    };
    ($r:ident, $name:literal, $base:literal, $state:ident, [$op:ident $(, $($a:tt)*)?], [$($b:tt)*]) => {
        $crate::__api_ingest_dispatch!($r, $name, $base, $state, [$($($a)*)?], [$($b)*]);
    };
    // INGEST stands alone: the collection takes batches and nothing else.
    ($r:ident, $name:literal, $base:literal, $state:ident, [], [$($b:tt)*]) => {
        $crate::__api_dispatch_ingest_only!($r, $name, $base, $state);
    };
}

/// The `POST` dispatcher where a single create is also routed.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_dispatch_with_create {
    ($r:ident, $name:literal, $base:literal, $state:ident) => {
        /// `POST` — one document, or a JSONL batch.
        ///
        /// Selected by `Content-Type`: `application/jsonl` (or
        /// `application/x-ndjson`) is a batch, anything else is the ordinary
        /// single create. One URL, two request shapes — so routing `INGEST`
        /// cannot change what an existing client sends.
        pub async fn create_or_ingest(
            state: ::axum::extract::State<$state>,
            ctx: ::axum::Extension<Context>,
            request: ::axum::extract::Request,
        ) -> ::axum::response::Response {
            use ::axum::response::IntoResponse as _;

            let content_type = request
                .headers()
                .get(::axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(::std::string::ToString::to_string);

            if $crate::jsonl::is_jsonl_request(content_type.as_deref()) {
                return ingest(state, ctx, request).await;
            }

            create(state, ctx, request).await.into_response()
        }
    };
}

/// The `POST` dispatcher where `INGEST` is the only create routed.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_dispatch_ingest_only {
    ($r:ident, $name:literal, $base:literal, $state:ident) => {
        /// `POST` — a JSONL batch, which is the only body this collection takes.
        pub async fn create_or_ingest(
            state: ::axum::extract::State<$state>,
            ::axum::Extension(ctx): ::axum::Extension<Context>,
            request: ::axum::extract::Request,
        ) -> ::axum::response::Response {
            use ::axum::response::IntoResponse as _;

            let content_type = request
                .headers()
                .get(::axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(::std::string::ToString::to_string);

            if !$crate::jsonl::is_jsonl_request(content_type.as_deref()) {
                // This collection routes no single create, so a document
                // body is not "invalid" — it is the wrong media type.
                return $crate::response::Errors::from(
                    $crate::JsonApiError::in_request($crate::ErrorCode::UnsupportedMediaType, &ctx)
                        .detail(::std::concat!(
                            "this collection accepts batches only; send `",
                            "application/jsonl",
                            "`"
                        )),
                )
                .into_response();
            }

            ingest(state, ::axum::Extension(ctx), request).await
        }
    };
}

/// The CREATE/CRUD half of [`__api_collection_post`].
#[doc(hidden)]
#[macro_export]
macro_rules! __api_plain_create_post {
    ($collection:ident, $m:ident, CREATE $(, $($rest:tt)*)?) => {
        $collection = $collection.post($m::create);
    };
    ($collection:ident, $m:ident, CRUD $(, $($rest:tt)*)?) => {
        $collection = $collection.post($m::create);
    };
    ($collection:ident, $m:ident, $p:ident :: $($rest:tt)+) => {
        $crate::__api_plain_create_post!($collection, $m, $($rest)+);
    };
    ($collection:ident, $m:ident, $op:ident $(, $($rest:tt)*)?) => {
        $crate::__api_plain_create_post!($collection, $m, $($($rest)*)?);
    };
    // Neither CREATE nor CRUD: the collection has no POST.
    ($collection:ident, $m:ident,) => {};
}

/// Emits a handler for each selected operation, and nothing for the rest.
///
/// This is what makes the operation traits opt-in: an unrouted verb generates
/// no handler, so its trait is never required.
#[doc(hidden)]
#[macro_export]
macro_rules! __api_handlers {
    ($r:ident, $name:literal, $base:literal, $state:ident $(,)?) => {};

    ($r:ident, $name:literal, $base:literal, $state:ident, $prefix:ident :: $($rest:tt)+) => {
        $crate::__api_handlers!($r, $name, $base, $state, $($rest)+);
    };

    ($r:ident, $name:literal, $base:literal, $state:ident, CRUD $(, $($rest:tt)*)?) => {
        $crate::__api_handlers!($r, $name, $base, $state, LIST, CREATE, READ, UPDATE, DELETE);
        $($crate::__api_handlers!($r, $name, $base, $state, $($rest)*);)?
    };

    ($r:ident, $name:literal, $base:literal, $state:ident, LIST $(, $($rest:tt)*)?) => {
        /// `GET` — one page of the collection.
        pub async fn list(
            ::axum::extract::State(state): ::axum::extract::State<$state>,
            ::axum::Extension(ctx): ::axum::Extension<Context>,
            uri: ::axum::http::Uri,
        ) -> ::std::result::Result<Fetched, Errors> {
            let query = $crate::query::Query::parse(uri.query().unwrap_or_default(), &ctx)?;

            $crate::crud::check_sort::<$state, $r>(&state, &query, &ctx)?;

            let event = $crate::hook::Event {
                operation: $crate::hook::Operation::List,
                resource: $name,
                id: ::std::option::Option::None,
            };
            $crate::crud::before(&state, &event, &ctx)?;

            let listing = match $crate::crud::List::<$r>::list(&state, &query, &ctx).await {
                ::std::result::Result::Ok(listing) => listing,
                ::std::result::Result::Err(err) => {
                    let errors = failed::<$state, $r>(&state, &ctx, "list", &err);
                    return $crate::crud::after(&state, &event, &ctx, ::std::result::Result::Err(errors));
                }
            };

            let mut resources = ::std::vec::Vec::with_capacity(listing.rows.len());
            for row in &listing.rows {
                resources.push(row.to_resource(&state, &ctx)?);
            }

            let links = $crate::query::Paginator::new($base, &query).links(
                listing.rows.len(),
                listing.total,
                listing.cursor.as_deref(),
                listing.has_more,
            );

            $crate::crud::after(
                &state,
                &event,
                &ctx,
                ::std::result::Result::Ok(
                    Fetched::page(resources, links, listing.total).in_context(&ctx),
                ),
            )
        }
        $($crate::__api_handlers!($r, $name, $base, $state, $($rest)*);)?
    };

    ($r:ident, $name:literal, $base:literal, $state:ident, CREATE $(, $($rest:tt)*)?) => {
        /// `POST` — create.
        pub async fn create(
            ::axum::extract::State(state): ::axum::extract::State<$state>,
            ::axum::Extension(ctx): ::axum::Extension<Context>,
            request: ::axum::extract::Request,
        ) -> ::std::result::Result<Created, Errors> {
            // Your FromRequest reads the body; what it produces is handed
            // straight to storage. It is an argument rather than a context
            // extension because a create always has a body — routing it
            // through the context would make an infallible step fallible.
            let event = $crate::hook::Event {
                operation: $crate::hook::Operation::Create,
                resource: $name,
                id: ::std::option::Option::None,
            };
            $crate::crud::before(&state, &event, &ctx)?;

            type Input = <$r as $crate::crud::Resource<$state>>::Create;
            let input =
                match <Input as FromRequest<$state>>::from_request(request, &state, &ctx).await {
                    ::std::result::Result::Ok(input) => input,
                    ::std::result::Result::Err(errors) => {
                        return $crate::crud::after(
                            &state, &event, &ctx, ::std::result::Result::Err(errors),
                        );
                    }
                };

            let created = match $crate::crud::Create::<$r>::create(&state, input, &ctx).await {
                ::std::result::Result::Ok(created) => created,
                ::std::result::Result::Err(err) => {
                    let errors = failed::<$state, $r>(&state, &ctx, "create", &err);
                    return $crate::crud::after(&state, &event, &ctx, ::std::result::Result::Err(errors));
                }
            };

            $crate::crud::after(
                &state,
                &event,
                &ctx,
                created.to_resource(&state, &ctx).map(Created).map_err(Errors::from),
            )
        }
        $($crate::__api_handlers!($r, $name, $base, $state, $($rest)*);)?
    };

    ($r:ident, $name:literal, $base:literal, $state:ident, READ $(, $($rest:tt)*)?) => {
        /// `GET /{id}` — fetch one.
        pub async fn show(
            ::axum::extract::State(state): ::axum::extract::State<$state>,
            ::axum::Extension(ctx): ::axum::Extension<Context>,
            ::axum::extract::Path(id): ::axum::extract::Path<::std::string::String>,
        ) -> ::std::result::Result<Fetched, Errors> {
            let event = $crate::hook::Event {
                operation: $crate::hook::Operation::Read,
                resource: $name,
                id: ::std::option::Option::Some(&id),
            };
            $crate::crud::before(&state, &event, &ctx)?;

            let found = match $crate::crud::Read::<$r>::get(&state, &id).await {
                ::std::result::Result::Ok(found) => found,
                ::std::result::Result::Err(err) => {
                    let errors = failed::<$state, $r>(&state, &ctx, "get", &err);
                    return $crate::crud::after(&state, &event, &ctx, ::std::result::Result::Err(errors));
                }
            };

            let outcome = found
                .ok_or_else(|| missing(&ctx, $name, &id))
                .and_then(|row| {
                    row.to_resource(&state, &ctx)
                        .map(|resource| Fetched::one(resource).in_context(&ctx))
                        .map_err(Errors::from)
                });

            $crate::crud::after(&state, &event, &ctx, outcome)
        }
        $($crate::__api_handlers!($r, $name, $base, $state, $($rest)*);)?
    };

    ($r:ident, $name:literal, $base:literal, $state:ident, UPDATE $(, $($rest:tt)*)?) => {
        /// `PATCH /{id}` — update.
        pub async fn update(
            ::axum::extract::State(state): ::axum::extract::State<$state>,
            ::axum::Extension(ctx): ::axum::Extension<Context>,
            ::axum::extract::Path(id): ::axum::extract::Path<::std::string::String>,
            request: ::axum::extract::Request,
        ) -> ::std::result::Result<Fetched, Errors> {
            let event = $crate::hook::Event {
                operation: $crate::hook::Operation::Update,
                resource: $name,
                id: ::std::option::Option::Some(&id),
            };
            $crate::crud::before(&state, &event, &ctx)?;

            type Input = <$r as $crate::crud::Resource<$state>>::Update;
            let input =
                match <Input as FromRequest<$state>>::from_request(request, &state, &ctx).await {
                    ::std::result::Result::Ok(input) => input,
                    ::std::result::Result::Err(errors) => {
                        return $crate::crud::after(
                            &state, &event, &ctx, ::std::result::Result::Err(errors),
                        );
                    }
                };

            // The path's id wins over anything the body says.
            let updated = match $crate::crud::Update::<$r>::update(&state, &id, input, &ctx).await {
                ::std::result::Result::Ok(updated) => updated,
                ::std::result::Result::Err(err) => {
                    let errors = failed::<$state, $r>(&state, &ctx, "update", &err);
                    return $crate::crud::after(&state, &event, &ctx, ::std::result::Result::Err(errors));
                }
            };

            $crate::crud::after(
                &state,
                &event,
                &ctx,
                updated
                    .to_resource(&state, &ctx)
                    .map(|resource| Fetched::one(resource).in_context(&ctx))
                    .map_err(Errors::from),
            )
        }
        $($crate::__api_handlers!($r, $name, $base, $state, $($rest)*);)?
    };

    ($r:ident, $name:literal, $base:literal, $state:ident, STREAM $(, $($rest:tt)*)?) => {
        /// `GET` with `Accept: application/jsonl` — the collection, streamed.
        ///
        /// Routed in place of `list` when STREAM is named, and falls back to
        /// it for any other `Accept`: one URL, two representations.
        pub async fn list_or_stream(
            ::axum::extract::State(state): ::axum::extract::State<$state>,
            ::axum::Extension(ctx): ::axum::Extension<Context>,
            headers: ::axum::http::HeaderMap,
            uri: ::axum::http::Uri,
        ) -> ::axum::response::Response {
            use ::axum::response::IntoResponse as _;

            let accept = headers
                .get(::axum::http::header::ACCEPT)
                .and_then(|value| value.to_str().ok());

            if !$crate::jsonl::wants_jsonl(accept) {
                // Not asked for: the document is what every other client gets.
                return list(
                    ::axum::extract::State(state),
                    ::axum::Extension(ctx),
                    uri,
                )
                .await
                .into_response();
            }

            let query = match $crate::query::Query::parse(uri.query().unwrap_or_default(), &ctx) {
                ::std::result::Result::Ok(query) => query,
                ::std::result::Result::Err(err) => {
                    return $crate::response::Errors::from(err).into_response();
                }
            };

            if let ::std::result::Result::Err(errors) =
                $crate::crud::check_sort::<$state, $r>(&state, &query, &ctx)
            {
                return errors.into_response();
            }

            let event = $crate::hook::Event {
                operation: $crate::hook::Operation::List,
                resource: $name,
                id: ::std::option::Option::None,
            };
            if let ::std::result::Result::Err(errors) =
                $crate::crud::before(&state, &event, &ctx)
            {
                return errors.into_response();
            }

            // The status is committed with the first byte, so the `after`
            // hook records the outcome now: a row that fails later becomes an
            // error *line*, not a different status.
            let outcome = $crate::crud::after(
                &state,
                &event,
                &ctx,
                ::std::result::Result::Ok::<(), $crate::response::Errors>(()),
            );
            if let ::std::result::Result::Err(errors) = outcome {
                return errors.into_response();
            }

            $crate::crud::stream_lines::<$state, $r>(state, query, ctx).into_response()
        }
        $($crate::__api_handlers!($r, $name, $base, $state, $($rest)*);)?
    };

    ($r:ident, $name:literal, $base:literal, $state:ident, INGEST $(, $($rest:tt)*)?) => {
        /// `POST` with `Content-Type: application/jsonl` — a streamed batch.
        ///
        /// One result line per input line. The status is `200` and is
        /// committed with the first byte, so a client reads outcomes from the
        /// lines rather than from the status — a batch in which every row
        /// failed is still a `200` whose every line is an error.
        pub async fn ingest(
            ::axum::extract::State(state): ::axum::extract::State<$state>,
            ::axum::Extension(ctx): ::axum::Extension<Context>,
            request: ::axum::extract::Request,
        ) -> ::axum::response::Response {
            use ::axum::response::IntoResponse as _;

            // Authorization is a batch-level decision: a denial must not
            // reach storage, and must not be re-asked per row.
            let event = $crate::hook::Event {
                operation: $crate::hook::Operation::Create,
                resource: $name,
                id: ::std::option::Option::None,
            };
            if let ::std::result::Result::Err(errors) =
                $crate::crud::before(&state, &event, &ctx)
            {
                return errors.into_response();
            }

            // The status is committed with the first byte, so the outcome is
            // recorded now; each line's own fate is recorded by the driver.
            let outcome = $crate::crud::after(
                &state,
                &event,
                &ctx,
                ::std::result::Result::Ok::<(), $crate::response::Errors>(()),
            );
            if let ::std::result::Result::Err(errors) = outcome {
                return errors.into_response();
            }

            let lines = $crate::extract::JsonlLines::new(request, &ctx);
            $crate::crud::ingest_lines::<$state, $r>(state, lines, ctx).into_response()
        }
        $($crate::__api_handlers!($r, $name, $base, $state, $($rest)*);)?
    };

    ($r:ident, $name:literal, $base:literal, $state:ident, CONTENT $(, $($rest:tt)*)?) => {
        /// `GET /{id}/content` — the resource's bytes.
        pub async fn content(
            ::axum::extract::State(state): ::axum::extract::State<$state>,
            ::axum::Extension(ctx): ::axum::Extension<Context>,
            ::axum::extract::Path(id): ::axum::extract::Path<::std::string::String>,
        ) -> ::std::result::Result<$crate::response::Download, Errors> {
            // Recorded as a read: it is one, and an audit trail that cannot
            // see who downloaded a file is missing the interesting half.
            let event = $crate::hook::Event {
                operation: $crate::hook::Operation::Read,
                resource: $name,
                id: ::std::option::Option::Some(&id),
            };
            $crate::crud::before(&state, &event, &ctx)?;

            let found = match $crate::crud::Content::<$r>::content(&state, &id).await {
                ::std::result::Result::Ok(found) => found,
                ::std::result::Result::Err(err) => {
                    let errors = failed::<$state, $r>(&state, &ctx, "content", &err);
                    return $crate::crud::after(&state, &event, &ctx, ::std::result::Result::Err(errors));
                }
            };

            let outcome = found.ok_or_else(|| missing(&ctx, $name, &id));
            $crate::crud::after(&state, &event, &ctx, outcome)
        }
        $($crate::__api_handlers!($r, $name, $base, $state, $($rest)*);)?
    };

    ($r:ident, $name:literal, $base:literal, $state:ident, DELETE $(, $($rest:tt)*)?) => {
        /// `DELETE /{id}` — destroy.
        pub async fn destroy(
            ::axum::extract::State(state): ::axum::extract::State<$state>,
            ::axum::Extension(ctx): ::axum::Extension<Context>,
            ::axum::extract::Path(id): ::axum::extract::Path<::std::string::String>,
        ) -> ::std::result::Result<NoContent, Errors> {
            let event = $crate::hook::Event {
                operation: $crate::hook::Operation::Delete,
                resource: $name,
                id: ::std::option::Option::Some(&id),
            };
            $crate::crud::before(&state, &event, &ctx)?;

            let outcome = $crate::crud::Delete::<$r>::destroy(&state, &id)
                .await
                .map(|()| NoContent)
                .map_err(|err| failed::<$state, $r>(&state, &ctx, "destroy", &err));

            $crate::crud::after(&state, &event, &ctx, outcome)
        }
        $($crate::__api_handlers!($r, $name, $base, $state, $($rest)*);)?
    };
}
