//! Turning documents and errors into HTTP responses.
//!
//! A handler returns [`Created`], [`Ok`] or a [`JsonApiError`] and gets the
//! right status, the JSON:API media type, and a correctly shaped body. None
//! of that is assembled at a call site.

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use std::collections::BTreeMap;

use crate::document::{Document, Links, MEDIA_TYPE, ResourceObject};
use crate::error::JsonApiError;

/// `200 OK` with a document.
///
/// Named `Fetched` rather than `Ok`: a handler returns
/// `Result<Fetched, Errors>`, and a type called `Ok` in that position
/// shadows `Result::Ok` at every call site.
pub struct Fetched(pub Document);

impl Fetched {
    /// One resource.
    pub fn one(resource: ResourceObject) -> Self {
        Self(Document::single(resource))
    }

    /// A collection.
    pub fn many(resources: Vec<ResourceObject>) -> Self {
        Self(Document::collection(resources))
    }

    /// One page of a collection, with its pagination links.
    ///
    /// Built by the generated `LIST` handler from what
    /// [`List::list`](crate::crud::List::list) returned. `total` becomes
    /// `meta.total` when storage counted the collection, so a client can size
    /// a pager without walking it.
    pub fn page(resources: Vec<ResourceObject>, links: Links, total: Option<usize>) -> Self {
        let mut document = Document::collection(resources).with_links(links);

        if let Some(total) = total {
            document.meta = Some(BTreeMap::from([(
                "total".to_string(),
                serde_json::Value::from(total),
            )]));
        }

        Self(document)
    }

    /// Record the negotiated locale on the document's top-level `meta`.
    ///
    /// A no-op when the request sent no `Accept-Language` this crate could
    /// parse, so a response carries `lang` only when there was one to carry.
    /// The rest of `meta` is left alone — a paged listing keeps its `total`.
    ///
    /// Applied by the generated handlers; call it on a hand-written one to
    /// get the same behaviour.
    #[must_use]
    pub fn in_context(self, ctx: &crate::Context) -> Self {
        match ctx.locale() {
            Some(locale) => Self(self.0.with_meta("lang", locale.as_str())),
            None => self,
        }
    }
}

/// `201 Created` with the resource and its `Location` header.
///
/// The location is the resource's own `self` link, so it is never spelled
/// separately.
pub struct Created(pub ResourceObject);

/// A JSONL body, written one line at a time.
///
/// Built by the generated `LIST` handler when the client sends
/// `Accept: application/jsonl` and the resource routes
/// [`STREAM`](crate::api!). Each row is serialized, written, and dropped, so
/// the response holds one row rather than the collection — see
/// [`jsonl`](crate::jsonl) for the trade this makes against a document.
#[cfg(feature = "streaming")]
#[cfg_attr(docsrs, doc(cfg(feature = "streaming")))]
pub struct Streamed(
    Box<dyn futures_core::Stream<Item = Result<Vec<u8>, std::convert::Infallible>> + Send + Unpin>,
);

#[cfg(feature = "streaming")]
impl Streamed {
    /// Build a JSONL body from a stream of already-rendered lines.
    ///
    /// Each item is one complete line, newline included. The error type is
    /// [`Infallible`](std::convert::Infallible) deliberately: a failure has
    /// to become a *line* rather than a transport error, because the status
    /// was already sent — see [`ERROR_LINE`](crate::jsonl::ERROR_LINE).
    pub fn new(
        lines: impl futures_core::Stream<Item = Result<Vec<u8>, std::convert::Infallible>>
        + Send
        + Unpin
        + 'static,
    ) -> Self {
        Self(Box::new(lines))
    }

    /// Render one resource object as a JSONL line.
    ///
    /// Serialization cannot fail for a `ResourceObject`, but if it somehow
    /// did, an error line is emitted rather than a truncated stream — a
    /// client must never mistake a partial export for a whole one.
    pub fn line(resource: &crate::document::ResourceObject) -> Vec<u8> {
        match serde_json::to_vec(resource) {
            Ok(mut line) => {
                line.push(b'\n');
                line
            }
            Err(_) => Self::error_line(&JsonApiError::new(crate::ErrorCode::Internal)),
        }
    }

    /// Render one created resource as an ingest result line.
    ///
    /// A *document* per line — `{"data":{…}}` — where [`line`](Self::line)
    /// writes a bare resource object. The difference is deliberate: an
    /// export is a row stream, but an ingest result line sits next to error
    /// lines that are themselves documents, so a client parses every line
    /// the same way and looks for `data` or `errors`.
    pub fn result_line(resource: &crate::document::ResourceObject) -> Vec<u8> {
        match serde_json::to_vec(&Document::single(resource.clone())) {
            Ok(mut line) => {
                line.push(b'\n');
                line
            }
            Err(_) => Self::error_line(&JsonApiError::new(crate::ErrorCode::Internal)),
        }
    }

    /// Render an error as one line's result, stamped with its line number.
    ///
    /// The line number is what makes a per-line failure actionable: a client
    /// that sent a hundred thousand rows needs to know *which* ones did not
    /// land, so it can retry exactly those. See
    /// [`LINE_MEMBER`](crate::jsonl::LINE_MEMBER).
    pub fn error_line_at(error: &JsonApiError, line: usize) -> Vec<u8> {
        let mut object = error.to_object();

        let meta = object.meta.get_or_insert_with(Default::default);
        meta.insert(
            crate::jsonl::LINE_MEMBER.to_string(),
            serde_json::Value::from(line),
        );

        let document = Document::errors(vec![object]);
        let mut rendered = serde_json::to_vec(&document).unwrap_or_else(|_| {
            br#"{"errors":[{"status":"500","code":"internal","title":"Internal server error"}]}"#
                .to_vec()
        });
        rendered.push(b'\n');
        rendered
    }

    /// Render an error as the stream's final line.
    ///
    /// Shaped like a JSON:API error document so a client parsing lines as
    /// JSON finds the same `errors` member it would in a normal response.
    pub fn error_line(error: &JsonApiError) -> Vec<u8> {
        let document = Document::errors(vec![error.to_object()]);
        let mut line = serde_json::to_vec(&document).unwrap_or_else(|_| {
            br#"{"errors":[{"status":"500","code":"internal","title":"Internal server error"}]}"#
                .to_vec()
        });
        line.push(b'\n');
        line
    }
}

#[cfg(feature = "streaming")]
impl IntoResponse for Streamed {
    fn into_response(self) -> Response {
        let body = axum::body::Body::from_stream(self.0);

        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, crate::jsonl::MEDIA_TYPE),
                // Two representations live at this URL, so a cache must key
                // on which one was asked for.
                (header::VARY, "Accept"),
            ],
            body,
        )
            .into_response()
    }
}

/// Raw bytes, with their own media type — a file, not a document.
///
/// The one response in this crate that is deliberately *not*
/// `application/vnd.api+json`. A resource's bytes are not a JSON:API
/// document and wrapping them in one would mean base64 in an attribute,
/// which doubles the transfer and cannot be streamed or ranged.
///
/// The conventional shape, and the one [`CONTENT`](crate::api!) routes: the
/// resource stays a JSON document carrying metadata at `/docs/{id}`, and its
/// bytes live beside it at `/docs/{id}/content`.
///
/// ```
/// use ash_jsonapi::response::Download;
///
/// let file = Download::new(vec![0x89, b'P', b'N', b'G'], "image/png")
///     .filename("diagram.png");
/// # let _ = file;
/// ```
pub struct Download {
    bytes: Vec<u8>,
    content_type: String,
    filename: Option<String>,
    inline: bool,
}

impl Download {
    /// Bytes, and the media type to serve them as.
    ///
    /// The media type is the caller's: this crate does not sniff content,
    /// because guessing a type from bytes is how an uploaded HTML file comes
    /// back as `text/html` and runs as the origin.
    pub fn new(bytes: impl Into<Vec<u8>>, content_type: impl Into<String>) -> Self {
        Self {
            bytes: bytes.into(),
            content_type: content_type.into(),
            filename: None,
            inline: false,
        }
    }

    /// Suggest a filename, and ask the browser to save rather than render.
    ///
    /// Sets `Content-Disposition: attachment`. The name is escaped — see
    /// [`disposition`].
    #[must_use]
    pub fn filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Render in the browser rather than downloading, keeping any filename.
    ///
    /// `Content-Disposition: inline` — for an image or a PDF a user expects
    /// to see rather than save. Off by default: `attachment` is the safer
    /// choice, since it does not invite the browser to interpret the bytes.
    #[must_use]
    pub fn inline(mut self) -> Self {
        self.inline = true;
        self
    }
}

impl IntoResponse for Download {
    fn into_response(self) -> Response {
        let mut headers = axum::http::HeaderMap::new();

        // A media type that will not parse as a header is the caller's bug,
        // not the client's; fall back to the neutral type rather than
        // sending a malformed header or panicking in a handler.
        let content_type = self
            .content_type
            .parse()
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("application/octet-stream"));
        headers.insert(header::CONTENT_TYPE, content_type);

        // Always sent, filename or not: it is what stops a browser from
        // interpreting an uploaded file as active content in the origin.
        if let std::result::Result::Ok(value) =
            disposition(self.inline, self.filename.as_deref()).parse()
        {
            headers.insert(header::CONTENT_DISPOSITION, value);
        }

        // Uploaded bytes are attacker-controlled, so the declared type must
        // be the only one considered — without this a browser may sniff an
        // `image/png` that is really HTML and run it.
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            axum::http::HeaderValue::from_static("nosniff"),
        );

        (StatusCode::OK, headers, self.bytes).into_response()
    }
}

/// Build a `Content-Disposition` value, safely.
///
/// A filename reaches this from an upload, so it is attacker-controlled and
/// cannot be interpolated as-is: a quote or a newline would end the header
/// value and start another. Two defences, per [RFC 6266]:
///
/// - the quoted form carries an ASCII-only fallback, with `"` and `\`
///   escaped and control characters dropped;
/// - `filename*` carries the real name, percent-encoded as UTF-8, which is
///   what a modern browser reads.
///
/// [RFC 6266]: https://www.rfc-editor.org/rfc/rfc6266#section-4.3
pub fn disposition(inline: bool, filename: Option<&str>) -> String {
    let kind = if inline { "inline" } else { "attachment" };

    let Some(name) = filename else {
        return kind.to_string();
    };

    // The ASCII fallback: printable ASCII only, quotes and backslashes
    // escaped. Anything else becomes `_` rather than being dropped, so the
    // name keeps its shape.
    let mut ascii = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            '"' | '\\' => {
                ascii.push('\\');
                ascii.push(ch);
            }
            c if c.is_ascii_graphic() || c == ' ' => ascii.push(c),
            _ => ascii.push('_'),
        }
    }

    // `filename*` in RFC 5987 form: UTF-8, percent-encoded, attribute
    // characters passed through.
    let mut encoded = String::with_capacity(name.len());
    for byte in name.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char);
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }

    format!("{kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

/// `204 No Content` — a successful delete.
pub struct NoContent;

impl IntoResponse for Fetched {
    fn into_response(self) -> Response {
        json_api(StatusCode::OK, &self.0, None)
    }
}

impl IntoResponse for Created {
    fn into_response(self) -> Response {
        let location = self.0.links.as_ref().and_then(|l| l.this.clone());

        json_api(StatusCode::CREATED, &Document::single(self.0), location)
    }
}

impl IntoResponse for NoContent {
    fn into_response(self) -> Response {
        (StatusCode::NO_CONTENT, [(header::CONTENT_TYPE, MEDIA_TYPE)]).into_response()
    }
}

/// Several errors in one response.
///
/// JSON:API allows a document to carry more than one error, and a validation
/// run usually produces several — a client fixing a form wants the whole list,
/// not the first problem. The status is the most severe of them.
///
/// `Debug` so a test can `.unwrap()` a `Result<_, Errors>`; the derive is on
/// [`JsonApiError`] already, and a handler's error type being un-debuggable
/// makes every test of one awkward for no benefit.
#[derive(Debug)]
pub struct Errors(pub Vec<JsonApiError>);

impl From<Vec<JsonApiError>> for Errors {
    fn from(errors: Vec<JsonApiError>) -> Self {
        Self(errors)
    }
}

impl From<JsonApiError> for Errors {
    fn from(error: JsonApiError) -> Self {
        Self(vec![error])
    }
}

impl IntoResponse for Errors {
    fn into_response(self) -> Response {
        // The most severe status wins: a 500 among 422s is still a 500.
        let status = self
            .0
            .iter()
            .map(|error| error.status())
            .max()
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        let objects = self.0.iter().map(JsonApiError::to_object).collect();

        // Every error in the batch came from the same request, so the first
        // one's locale is the request's.
        let document = with_lang(Document::errors(objects), self.0.first());
        json_api(status, &document, None)
    }
}

/// An error is a response: `return err.into()` from any handler.
impl IntoResponse for JsonApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let document = with_lang(Document::errors(vec![self.to_object()]), Some(&self));
        json_api(status, &document, None)
    }
}

/// Record the request's locale on an error document, when it had one.
fn with_lang(document: Document, error: Option<&JsonApiError>) -> Document {
    match error.and_then(JsonApiError::locale) {
        Some(locale) => document.with_meta("lang", locale.as_str()),
        None => document,
    }
}

fn json_api(status: StatusCode, doc: &Document, location: Option<String>) -> Response {
    let body = serde_json::to_string(doc).unwrap_or_else(|_| {
        // A Document is always serializable; this keeps the signature
        // infallible rather than panicking in a handler.
        r#"{"errors":[{"status":"500","code":"internal","title":"Internal server error"}]}"#
            .to_string()
    });

    let mut response = (status, [(header::CONTENT_TYPE, MEDIA_TYPE)], body).into_response();
    if let Some(location) = location
        && let std::result::Result::Ok(value) = location.parse()
    {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}
