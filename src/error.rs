//! JSON:API error objects and their mapping onto HTTP status codes.
//!
//! Every error a handler returns is one of these: a code, the HTTP status it
//! maps to, and a pointer at whatever caused it. Internal errors are
//! sanitized — the detail is dropped and only the correlation id survives, so
//! a connection string in a data-layer error cannot reach a client.

use crate::context::Context;
use crate::document::{ErrorObject, ErrorSource};
pub use http_types::StatusCode;

/// The JSON:API `code` values this crate emits, and the HTTP status each
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// `UnknownResource` / `UnknownAction` / `NotFound`.
    NotFound,
    /// A deliberate policy denial.
    Forbidden,
    /// A policy *outage* — distinct from a denial, and retriable.
    PolicyUnavailable,
    MissingTenant,
    /// A validation failure on a specific attribute.
    InvalidAttribute,
    /// A malformed or unknown query parameter — an unknown sort attribute,
    /// filter key or include path, or two pagination strategies at once.
    /// Resolved against the schema before the data layer is touched.
    InvalidParameter,
    /// Optimistic-concurrency failure; retriable.
    Conflict,
    /// The `If-Match` precondition was supplied and stale.
    PreconditionFailed,
    LockContention,
    /// The domain is draining. A lifecycle signal, not a fault.
    Closing,
    Unsupported,
    /// The request body was not sent as `application/vnd.api+json`.
    ///
    /// JSON:API requires the media type on any request carrying a document,
    /// and requires `415` when it is missing — a body sent as
    /// `application/json` is refused rather than guessed at.
    UnsupportedMediaType,
    /// `DataLayer` / `Serialization`. The message is never surfaced.
    Internal,
}

impl ErrorCode {
    /// The HTTP status for this code.
    pub fn status(self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::PolicyUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::MissingTenant => StatusCode::BAD_REQUEST,
            Self::InvalidAttribute => StatusCode::UNPROCESSABLE_ENTITY,
            Self::InvalidParameter => StatusCode::BAD_REQUEST,
            Self::Conflict => StatusCode::CONFLICT,
            Self::PreconditionFailed => StatusCode::PRECONDITION_FAILED,
            Self::LockContention => StatusCode::SERVICE_UNAVAILABLE,
            Self::Closing => StatusCode::SERVICE_UNAVAILABLE,
            Self::Unsupported => StatusCode::NOT_IMPLEMENTED,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The wire value of the JSON:API `code` member.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Forbidden => "forbidden",
            Self::PolicyUnavailable => "policy_unavailable",
            Self::MissingTenant => "missing_tenant",
            Self::InvalidAttribute => "invalid_attribute",
            Self::InvalidParameter => "invalid_parameter",
            Self::Conflict => "conflict",
            Self::PreconditionFailed => "precondition_failed",
            Self::LockContention => "lock_contention",
            Self::Closing => "closing",
            Self::Unsupported => "unsupported",
            Self::UnsupportedMediaType => "unsupported_media_type",
            Self::Internal => "internal",
        }
    }

    /// A short, human-readable title. Safe to show a client.
    ///
    /// Not `StatusCode::canonical_reason()`: three of these codes are 503,
    /// and "Service Unavailable" would collapse the distinction between a
    /// policy outage, lock contention and a draining server that the codes exist
    /// to preserve.
    pub fn title(self) -> &'static str {
        match self {
            Self::NotFound => "Not found",
            Self::Forbidden => "Forbidden",
            Self::PolicyUnavailable => "Policy unavailable",
            Self::MissingTenant => "Missing tenant",
            Self::InvalidAttribute => "Invalid attribute",
            Self::InvalidParameter => "Invalid query parameter",
            Self::Conflict => "Conflict",
            Self::PreconditionFailed => "Precondition failed",
            Self::LockContention => "Lock contention",
            Self::Closing => "Server is shutting down",
            Self::Unsupported => "Unsupported",
            Self::UnsupportedMediaType => "Unsupported media type",
            Self::Internal => "Internal server error",
        }
    }

    /// Whether the client may retry. Drives `Retry-After`.
    pub fn is_retriable(self) -> bool {
        matches!(
            self,
            Self::PolicyUnavailable | Self::LockContention | Self::Closing | Self::Conflict
        )
    }

    /// Whether the underlying error's message must be withheld from the
    /// response body.
    pub fn is_sanitized(self) -> bool {
        matches!(self, Self::Internal)
    }
}

/// The detail text sent in place of a sanitized internal error message.
pub const INTERNAL_DETAIL: &str = "An internal error occurred.";

/// An error on its way to becoming a JSON:API error object.
#[derive(Debug, Clone)]
pub struct JsonApiError {
    pub code: ErrorCode,
    detail: Option<String>,
    source: Option<ErrorSource>,
    correlation_id: Option<String>,
    /// Boxed because it is the rare case: a `JsonApiError` is the `Err` of
    /// most `Result`s in this crate, and a `Translation` inline would widen
    /// every one of them for a field few errors set.
    translation: Option<Box<crate::i18n::Translation>>,
    /// The locale the request asked for, taken from the context so an error
    /// document can carry `meta.lang` like any other response.
    ///
    /// Boxed for the same reason as `translation`: this type is the `Err` of
    /// most `Result`s in the crate, and most requests send no
    /// `Accept-Language`.
    locale: Option<Box<crate::i18n::Locale>>,
}

impl JsonApiError {
    /// An error in a request's context.
    ///
    /// The usual constructor: every error a handler returns should carry the
    /// correlation id, so taking the context up front is one call rather than
    /// a `.in_context(ctx)` remembered at each site.
    pub fn in_request(code: ErrorCode, ctx: &Context) -> Self {
        Self::new(code).in_context(ctx)
    }

    /// A `500` that says nothing.
    ///
    /// The detail is dropped by [`ErrorCode::is_sanitized`], so the cause has
    /// to be logged separately; the correlation id is what connects the two.
    pub fn internal(ctx: &Context) -> Self {
        Self::in_request(ErrorCode::Internal, ctx)
    }

    /// A `422` pointing at one member of the request document.
    pub fn invalid(ctx: &Context, detail: impl Into<String>, pointer: impl Into<String>) -> Self {
        Self::in_request(ErrorCode::InvalidAttribute, ctx)
            .detail(detail)
            .pointer(pointer)
    }

    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            detail: None,
            source: None,
            correlation_id: None,
            translation: None,
            locale: None,
        }
    }

    /// Attach client-facing detail.
    ///
    /// Ignored for codes that must be sanitized — an internal error's message
    /// can carry a connection string, so this cannot become a leak by
    /// accident at a call site.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        if !self.code.is_sanitized() {
            self.detail = Some(detail.into());
        }
        self
    }

    /// Point at the offending attribute.
    ///
    /// A dotted field name becomes a nested JSON pointer:
    /// `address.city` → `/data/attributes/address/city`.
    pub fn attribute(mut self, field: &str) -> Self {
        let pointer = format!("/data/attributes/{}", field.replace('.', "/"));
        self.source = Some(ErrorSource::pointer(pointer));
        self
    }

    /// Point at any member by JSON pointer, e.g.
    /// `/data/relationships/owner/data`.
    ///
    /// [`Self::attribute`] covers the common case; this is for members
    /// outside `/data/attributes`, which relationships and the document
    /// envelope both are.
    #[must_use]
    pub fn pointer(mut self, pointer: impl Into<String>) -> Self {
        self.source = Some(ErrorSource::pointer(pointer));
        self
    }

    /// Point at the offending query parameter, e.g. `sort`.
    pub fn parameter(mut self, param: impl Into<String>) -> Self {
        self.source = Some(ErrorSource::parameter(param));
        self
    }

    /// Attach a translation key, so a client can render this failure in its
    /// own language.
    ///
    /// The [`detail`](Self::detail) stays as it is — it is the fallback for a
    /// client with no catalog — and the key travels beside it under the error
    /// object's `meta.i18n`. See [`i18n`](crate::i18n) for why the server
    /// sends a key rather than translated prose.
    ///
    /// ```
    /// use ash_jsonapi::i18n::Translation;
    /// use ash_jsonapi::{Context, ErrorCode, JsonApiError};
    ///
    /// # let ctx = Context::new("");
    /// let err = JsonApiError::in_request(ErrorCode::InvalidAttribute, &ctx)
    ///     .detail("must be 80 characters or fewer")
    ///     .attribute("name")
    ///     .translate(Translation::new("todo.name.too_long").param("max", 80));
    ///
    /// let meta = err.to_object().meta.unwrap();
    /// assert_eq!(meta["i18n"]["key"], "todo.name.too_long");
    /// ```
    ///
    /// Unlike [`detail`](Self::detail), a key is kept on a sanitized code: it
    /// names a message this crate chose, not the underlying cause, so it
    /// cannot leak what `is_sanitized` exists to withhold.
    #[must_use]
    pub fn translate(mut self, translation: crate::i18n::Translation) -> Self {
        self.translation = Some(Box::new(translation));
        self
    }

    /// The translation key attached to this error, if any.
    pub fn translation(&self) -> Option<&crate::i18n::Translation> {
        self.translation.as_deref()
    }

    /// Set the correlation id — what makes "here is my error id" resolvable
    /// in the audit log.
    pub fn correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Take the correlation id from the request's [`Context`].
    ///
    /// The normal way to build an error in a handler: the id is already in
    /// the context, so it never has to be passed separately or remembered.
    #[must_use]
    pub fn in_context(mut self, ctx: &Context) -> Self {
        self.locale = ctx.locale().cloned().map(Box::new);
        self.correlation_id(ctx.correlation_id())
    }

    /// The locale this error was built in, if the request named one.
    pub fn locale(&self) -> Option<&crate::i18n::Locale> {
        self.locale.as_deref()
    }

    /// The HTTP status to respond with.
    pub fn status(&self) -> StatusCode {
        self.code.status()
    }

    /// Render the JSON:API error object.
    pub fn to_object(&self) -> ErrorObject {
        ErrorObject {
            id: self.correlation_id.clone(),
            status: self.code.status().as_u16().to_string(),
            code: self.code.as_str().to_string(),
            title: self.code.title().to_string(),
            detail: if self.code.is_sanitized() {
                Some(INTERNAL_DETAIL.to_string())
            } else {
                self.detail.clone()
            },
            source: self.source.clone(),
            meta: self.meta(),
        }
    }

    /// The error object's `meta`, or `None` when there is nothing to put in
    /// it — an absent member rather than an empty object.
    fn meta(&self) -> Option<std::collections::BTreeMap<String, serde_json::Value>> {
        let translation = self.translation.as_deref()?;
        let encoded = serde_json::to_value(translation).ok()?;

        Some(std::collections::BTreeMap::from([(
            "i18n".to_string(),
            encoded,
        )]))
    }
}

impl std::fmt::Display for JsonApiError {
    /// The title, plus the detail when there is one the client may see.
    ///
    /// A sanitized code has no detail to show, so it prints its title alone —
    /// the same text the client receives, which keeps a log line and a
    /// response consistent about what was said.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{}: {detail}", self.code.title()),
            None => f.write_str(self.code.title()),
        }
    }
}

impl std::error::Error for JsonApiError {}
