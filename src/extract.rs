//! Getting a request's body into your own types.
//!
//! A handler should receive one [`Context`] carrying everything the request
//! turned into, not a widening tuple of extractors. [`FromRequest`] is where
//! you say how that happens: read the body however you like, produce your
//! type, and the generated routes attach it to the context for you.
//!
//! ```text
//! impl FromRequest<App> for NewTodo {
//!     async fn from_request(req: Request, app: &App, ctx: &Context) -> Result<Self, Errors> {
//!         // The JSON:API path: validate against the resource's schema, then
//!         // map the document. `validated` does the first half.
//!         let doc = validated(req, ctx, app.validator("todo")).await?;
//!         Ok(NewTodo { message: doc["data"]["attributes"]["message"].as_str()… })
//!     }
//! }
//! ```
//!
//! Nothing here assumes JSON. A resource whose create body is multipart, or
//! form-encoded, or a raw upload, implements the same trait and reads the
//! request its own way.

use axum::extract::Request;

use crate::context::Context;
use crate::document::MEDIA_TYPE;
use crate::error::{ErrorCode, JsonApiError};
use crate::response::Errors;

/// Build `Self` from a request.
///
/// Implemented on the type a handler wants — the parsed body, an
/// authenticated actor, a resolved tenant. Whatever it produces is attached
/// to the [`Context`] with [`Context::with`], so downstream code reads it
/// back with [`Context::extract`] rather than threading a parameter.
///
/// The `Context` passed in is the request's own, so errors can carry the
/// correlation id: build them with [`JsonApiError::invalid`] and friends.
pub trait FromRequest<S>: Sized + Send + Sync + 'static {
    /// Read the request into `Self`.
    ///
    /// Consumes the request, since reading a body is a one-shot operation.
    /// `state` is your application state, so an implementation can reach the
    /// validator, a connection pool, or anything else it needs.
    fn from_request(
        request: Request,
        state: &S,
        ctx: &Context,
    ) -> impl Future<Output = Result<Self, Errors>> + Send;
}

/// Check that a request carries a JSON:API body.
///
/// JSON:API requires `application/vnd.api+json` on a request with a document,
/// and `415` when it is absent, so a body sent as `application/json` is
/// refused rather than guessed at. Parameters after the media type are
/// allowed through, since a charset is common in the wild.
///
/// Called by [`body_text`], so every implementation built on it gets the
/// check. Call it directly only in a `FromRequest` that reads the body some
/// other way.
pub fn check_media_type(request: &Request, ctx: &Context) -> Result<(), Errors> {
    let content_type = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    // `application/vnd.api+json; charset=utf-8` is the same media type.
    let media_type = content_type.split(';').next().unwrap_or_default().trim();

    if media_type.eq_ignore_ascii_case(MEDIA_TYPE) {
        return Ok(());
    }

    Err(
        JsonApiError::in_request(ErrorCode::UnsupportedMediaType, ctx)
            .detail(format!(
                "requests with a body must be sent as `{MEDIA_TYPE}`, not `{media_type}`"
            ))
            .into(),
    )
}

/// Read a request body as UTF-8 text.
///
/// The starting point for most implementations. The media type is checked
/// first — a body that is not `application/vnd.api+json` is a `415` — and a
/// body that is not valid UTF-8 is a `422`, both before any parsing.
pub async fn body_text(request: Request, ctx: &Context) -> Result<String, Errors> {
    check_media_type(&request, ctx)?;

    let bytes = axum::body::to_bytes(request.into_body(), MAX_BODY)
        .await
        .map_err(|err| {
            // A body that cannot be read is the client's problem — a
            // disconnect, or one larger than the cap below.
            JsonApiError::invalid(ctx, format!("could not read the request body: {err}"), "")
        })?;

    String::from_utf8(bytes.to_vec())
        .map_err(|_| JsonApiError::invalid(ctx, "the request body is not valid UTF-8", "").into())
}

/// The largest body [`body_text`] will read: 2 MiB.
///
/// A cap is required — without one a client can ask the server to buffer
/// until it dies. Read the body yourself if a resource legitimately needs
/// more.
pub const MAX_BODY: usize = 2 * 1024 * 1024;

/// Read a body and check it against a resource's request schema.
///
/// The JSON:API half of [`FromRequest`]: the body is parsed as JSON and
/// validated against the [`DocumentValidator`](crate::validation::DocumentValidator)
/// registered for `resource`, so
/// every structural failure — a missing member, a wrong `type`, an attribute
/// of the wrong type, malformed linkage — comes back as a complete error
/// document with a JSON Pointer at each cause.
///
/// What is left for the caller is the part a schema cannot express: mapping
/// the validated document onto its own type.
///
/// ```ignore
/// impl FromRequest<App> for NewTodo {
///     async fn from_request(req: Request, app: &App, ctx: &Context) -> Result<Self, Errors> {
///         let doc = validated(req, ctx, app.validator("todo")).await?;
///         Ok(NewTodo { message: doc["data"]["attributes"]["message"].as_str().unwrap().into() })
///     }
/// }
/// ```
#[cfg(feature = "validation")]
#[cfg_attr(docsrs, doc(cfg(feature = "validation")))]
pub async fn validated(
    request: Request,
    ctx: &Context,
    validator: &crate::validation::DocumentValidator,
) -> Result<serde_json::Value, Errors> {
    let body = body_text(request, ctx).await?;
    validator.parse(&body, ctx).map_err(Errors)
}

/// The `id` a document addresses, from the request path.
///
/// A `PATCH` addresses a row that already exists, and the path's id is
/// authoritative over anything the body says — so this is what an update's
/// [`FromRequest`] should use rather than reading `data.id`.
pub fn path_id(request: &Request) -> Option<&str> {
    request
        .extensions()
        .get::<axum::extract::Path<String>>()
        .map(|axum::extract::Path(id)| id.as_str())
}

/// A raw request body — an uploaded file, rather than a JSON:API document.
///
/// The counterpart of [`Download`](crate::response::Download) on the way in.
/// Use it as a resource's [`Create`](crate::crud::Resource::Create) input
/// when the body *is* the file:
///
/// ```ignore
/// impl Resource<App> for Doc {
///     type Create = Upload;
///     type Update = Upload;
///     // …
/// }
///
/// impl Create<Doc> for App {
///     async fn create(&self, input: Upload, ctx: &Context) -> Result<Doc, Self::Error> {
///         Ok(self.store(input.content_type, input.bytes))
///     }
/// }
/// ```
///
/// # Limits
///
/// The default cap is [`MAX_UPLOAD`], not [`MAX_BODY`]: a file is expected to
/// be larger than a document. Both exist because an unbounded read is a
/// denial of service written in one line — a client that can make the server
/// buffer without limit does not need any other exploit.
///
/// Raise or lower it per resource with [`Upload::limited`], and restrict what
/// may be sent with [`Upload::accepting`].
#[derive(Debug, Clone)]
pub struct Upload {
    /// The bytes, as sent.
    pub bytes: Vec<u8>,
    /// The `Content-Type` the client declared.
    ///
    /// Taken at the client's word and never sniffed — but see
    /// [`Upload::accepting`], and note that
    /// [`Download`](crate::response::Download) sends `nosniff` so a lie here
    /// cannot become active content in your origin.
    pub content_type: String,
}

/// The largest [`Upload`] read by default: 10 MiB.
pub const MAX_UPLOAD: usize = 10 * 1024 * 1024;

/// The type assumed when a request declares none.
const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

impl Upload {
    /// Read a request body as an upload, capped at `limit` bytes.
    ///
    /// The building block behind the [`FromRequest`] impl; call it directly
    /// from your own impl to use a different limit:
    ///
    /// ```ignore
    /// impl FromRequest<App> for Avatar {
    ///     async fn from_request(req: Request, _s: &App, ctx: &Context) -> Result<Self, Errors> {
    ///         let upload = Upload::limited(req, ctx, 256 * 1024).await?;
    ///         Ok(Avatar(upload))
    ///     }
    /// }
    /// ```
    pub async fn limited(request: Request, ctx: &Context, limit: usize) -> Result<Self, Errors> {
        let content_type = request
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or(DEFAULT_CONTENT_TYPE)
            .to_string();

        let bytes = axum::body::to_bytes(request.into_body(), limit)
            .await
            .map_err(|_| {
                // Almost always the cap: `to_bytes` refuses a body over the
                // limit, and saying so names the actual problem instead of
                // reporting a generic read failure.
                JsonApiError::in_request(ErrorCode::InvalidAttribute, ctx).detail(format!(
                    "the uploaded body could not be read, or exceeds the {limit}-byte limit"
                ))
            })?;

        Ok(Self {
            bytes: bytes.to_vec(),
            content_type,
        })
    }

    /// Read an upload, refusing a media type not in `accepted`.
    ///
    /// The check a file endpoint should not be without: an avatar route that
    /// accepts `text/html` is a stored-XSS vector waiting for someone to
    /// serve it back inline. Compared against the type alone, so
    /// `image/png; charset=…` still matches `image/png`.
    pub async fn accepting(
        request: Request,
        ctx: &Context,
        limit: usize,
        accepted: &[&str],
    ) -> Result<Self, Errors> {
        let upload = Self::limited(request, ctx, limit).await?;

        // Parameters after `;` are not part of the type.
        let kind = upload
            .content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim();

        if !accepted
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(kind))
        {
            return Err(
                JsonApiError::in_request(ErrorCode::UnsupportedMediaType, ctx)
                    .detail(format!(
                        "`{kind}` is not accepted here; send one of {}",
                        accepted
                            .iter()
                            .map(|allowed| format!("`{allowed}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .into(),
            );
        }

        Ok(upload)
    }

    /// How many bytes were uploaded.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the body was empty.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Reads any request body as an upload, capped at [`MAX_UPLOAD`].
///
/// Deliberately does *not* check the media type — an upload is not a JSON:API
/// document, so the JSON:API media-type check would reject every real file. Use
/// [`Upload::accepting`] from your own impl when the endpoint should only
/// take certain types.
impl<S: Sync> FromRequest<S> for Upload {
    async fn from_request(request: Request, _state: &S, ctx: &Context) -> Result<Self, Errors> {
        Self::limited(request, ctx, MAX_UPLOAD).await
    }
}

/// The largest single line [`JsonlLines`] will read: 1 MiB.
///
/// The cap that matters for an ingest. The *body* is deliberately unbounded —
/// that is the point of streaming one — so the only thing standing between a
/// client and unbounded buffering is a limit on how much may arrive with no
/// newline in it. Without this, `{"a":` repeated forever is a memory
/// exhaustion attack written in one line.
#[cfg(feature = "streaming")]
pub const MAX_LINE: usize = 1024 * 1024;

/// A request body, split into lines as it arrives.
///
/// The ingest counterpart of [`Streamed`](crate::response::Streamed): that
/// writes one resource per line, this reads one document per line. Neither
/// holds more than a line at a time, so a batch of a million rows costs a
/// line of memory rather than a million.
///
/// Yields the 1-based line number alongside each line, because a per-line
/// result is only actionable if the client can map it back onto its input —
/// see [`LINE_MEMBER`](crate::jsonl::LINE_MEMBER).
///
/// Blank lines are skipped rather than reported: a trailing newline is how
/// every JSONL writer ends a file, and calling that a parse error would fail
/// almost every well-formed batch.
///
/// # Failing
///
/// Two failures end the stream, both before any row is written: a transport
/// error, and a line over [`MAX_LINE`]. Both arrive as an `Err` item carrying
/// the line number, so the handler can render it as that line's result.
#[cfg(feature = "streaming")]
#[cfg_attr(docsrs, doc(cfg(feature = "streaming")))]
pub struct JsonlLines {
    body: axum::body::BodyDataStream,
    /// Bytes read but not yet terminated by a newline.
    buffer: Vec<u8>,
    /// How many lines have been yielded, so the next carries the right number.
    line: usize,
    limit: usize,
    ctx: Context,
    done: bool,
}

#[cfg(feature = "streaming")]
impl JsonlLines {
    /// Read a request body as lines, capped at [`MAX_LINE`] per line.
    pub fn new(request: Request, ctx: &Context) -> Self {
        Self::limited(request, ctx, MAX_LINE)
    }

    /// Read a request body as lines, capped at `limit` bytes per line.
    pub fn limited(request: Request, ctx: &Context, limit: usize) -> Self {
        Self {
            body: request.into_body().into_data_stream(),
            buffer: Vec::new(),
            line: 0,
            limit,
            ctx: ctx.clone(),
            done: false,
        }
    }

    /// Take the next complete line out of the buffer, if there is one.
    fn take_line(&mut self) -> Option<Vec<u8>> {
        let newline = self.buffer.iter().position(|byte| *byte == b'\n')?;

        // `drain` leaves the remainder in place, so a frame carrying several
        // lines is split without reallocating the tail each time.
        let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
        line.pop();
        // A CRLF writer leaves the carriage return behind.
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        Some(line)
    }

    /// The error for a line that exceeded the cap.
    fn too_long(&self, line: usize) -> JsonApiError {
        JsonApiError::in_request(ErrorCode::InvalidAttribute, &self.ctx)
            .detail(format!(
                "line {line} exceeds the {} -byte limit",
                self.limit
            ))
            .pointer("")
    }
}

#[cfg(feature = "streaming")]
impl futures_core::Stream for JsonlLines {
    /// The line number, and either its bytes or why it could not be read.
    type Item = (usize, Result<Vec<u8>, JsonApiError>);

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;

        loop {
            if self.done {
                return Poll::Ready(None);
            }

            // A complete line already buffered is yielded without touching
            // the body: one frame often carries many lines.
            if let Some(line) = self.take_line() {
                self.line += 1;
                let number = self.line;

                // Blank lines are structural, not data — skip and continue.
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                return Poll::Ready(Some((number, Ok(line))));
            }

            // Nothing buffered is a whole line, so the partial one must not
            // be allowed to grow without bound.
            if self.buffer.len() > self.limit {
                self.done = true;
                let number = self.line + 1;
                let error = self.too_long(number);
                return Poll::Ready(Some((number, Err(error))));
            }

            match std::pin::Pin::new(&mut self.body).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(frame))) => self.buffer.extend_from_slice(&frame),
                Poll::Ready(Some(Err(_))) => {
                    // The client went away, or the body was truncated. There
                    // is no line to blame, so the next one is named.
                    self.done = true;
                    self.line += 1;
                    let number = self.line;
                    let error = JsonApiError::in_request(ErrorCode::InvalidAttribute, &self.ctx)
                        .detail("the request body could not be read to the end")
                        .pointer("");
                    return Poll::Ready(Some((number, Err(error))));
                }
                Poll::Ready(None) => {
                    self.done = true;

                    // A final line with no trailing newline is still a line.
                    let rest = std::mem::take(&mut self.buffer);
                    if rest.iter().all(u8::is_ascii_whitespace) {
                        return Poll::Ready(None);
                    }
                    self.line += 1;
                    return Poll::Ready(Some((self.line, Ok(rest))));
                }
            }
        }
    }
}
