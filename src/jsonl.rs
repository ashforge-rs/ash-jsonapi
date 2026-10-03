//! JSON Lines: one resource per line, streamed as rows arrive.
//!
//! A JSON:API collection document is a single value: the closing bracket
//! cannot be written until the last row is known, so the whole page is held
//! in memory and the client waits for all of it. That is the right trade for
//! a page of twenty-five. It is the wrong one for an export of a million
//! rows, where the document never fits in memory on either end.
//!
//! JSONL inverts it. Each line is a complete resource object, so the server
//! writes a row and forgets it, and the client parses a row and acts on it —
//! neither holds more than one at a time, and the first row arrives in
//! milliseconds rather than after the last one is found.
//!
//! ```text
//! {"type":"todo","id":"t_1","attributes":{"message":"buy milk"}}
//! {"type":"todo","id":"t_2","attributes":{"message":"write docs"}}
//! ```
//!
//! # Asking for it
//!
//! `Accept: application/jsonl` on the ordinary collection URL. There is no
//! second route: the collection is one resource, and how it is rendered is
//! what `Accept` is for — so a cache keyed on `Vary: Accept` already knows
//! these are two representations of one thing.
//!
//! Anything else — including no `Accept` at all — gets the JSON:API document,
//! so this cannot change what an existing client receives.
//!
//! # What is lost
//!
//! A JSONL response carries no `links`, no `meta` and no `included`: there is
//! no document to hang them on. It is a row stream, not a document, and a
//! client that needs pagination links should ask for the document instead.
//! Errors are the exception — see [`ERROR_LINE`].

/// The media type: `application/jsonl`.
///
/// `application/x-ndjson` is the other name in the wild and is accepted on
/// the way in, but this is what is sent back.
pub const MEDIA_TYPE: &str = "application/jsonl";

/// The other spelling clients use for the same thing.
pub const MEDIA_TYPE_NDJSON: &str = "application/x-ndjson";

/// Whether an `Accept` header asks for JSONL.
///
/// True when JSONL is named *and* is at least as preferred as any JSON:API
/// alternative in the same header. A client that sends
/// `application/vnd.api+json, application/jsonl;q=0.5` prefers the document
/// and gets it; one that sends only `application/jsonl` gets lines.
///
/// ```
/// use ash_jsonapi::jsonl::wants_jsonl;
///
/// assert!(wants_jsonl(Some("application/jsonl")));
/// assert!(wants_jsonl(Some("application/x-ndjson")));
///
/// // The document is the default, in every ambiguous case.
/// assert!(!wants_jsonl(None));
/// assert!(!wants_jsonl(Some("*/*")));
/// assert!(!wants_jsonl(Some("application/vnd.api+json")));
///
/// // Explicit `q` decides when both are named.
/// assert!(!wants_jsonl(Some("application/vnd.api+json, application/jsonl;q=0.5")));
/// assert!(wants_jsonl(Some("application/vnd.api+json;q=0.2, application/jsonl")));
/// ```
pub fn wants_jsonl(accept: Option<&str>) -> bool {
    let Some(accept) = accept else {
        return false;
    };

    let mut jsonl = None::<f32>;
    let mut document = None::<f32>;

    for entry in accept.split(',') {
        let mut parts = entry.split(';');
        let Some(kind) = parts.next().map(str::trim) else {
            continue;
        };

        let quality = parts
            .find_map(|part| part.trim().strip_prefix("q=")?.trim().parse::<f32>().ok())
            .unwrap_or(1.0);

        // `q=0` means "not acceptable", so it never selects anything.
        if quality <= 0.0 {
            continue;
        }

        let slot = if kind.eq_ignore_ascii_case(MEDIA_TYPE)
            || kind.eq_ignore_ascii_case(MEDIA_TYPE_NDJSON)
        {
            &mut jsonl
        } else if kind.eq_ignore_ascii_case(crate::document::MEDIA_TYPE) {
            &mut document
        } else {
            // `*/*` and everything else leave the default alone: a client
            // that expresses no preference gets the document.
            continue;
        };

        if slot.is_none_or(|best| quality > best) {
            *slot = Some(quality);
        }
    }

    match (jsonl, document) {
        (None, _) => false,
        // Named alone, or preferred over the document. Ties go to JSONL,
        // since naming it at all is a deliberate act.
        (Some(lines), None) => lines > 0.0,
        (Some(lines), Some(doc)) => lines >= doc,
    }
}

/// Whether a request body is JSONL, from its `Content-Type`.
///
/// The ingest-side counterpart of [`wants_jsonl`]: that one reads `Accept`
/// and decides how to *render* a collection, this one reads `Content-Type`
/// and decides how to *read* a body. Both spellings are accepted, and
/// parameters after the type are ignored, so `application/jsonl;
/// charset=utf-8` is the same media type.
///
/// A `POST` that is not JSONL is an ordinary single-resource create — which
/// is what keeps [`INGEST`](crate::api!) additive: routing it cannot change
/// what an existing client sends.
///
/// ```
/// use ash_jsonapi::jsonl::is_jsonl_request;
///
/// assert!(is_jsonl_request(Some("application/jsonl")));
/// assert!(is_jsonl_request(Some("application/x-ndjson")));
/// assert!(is_jsonl_request(Some("application/jsonl; charset=utf-8")));
///
/// // Anything else is a single create, including the JSON:API media type.
/// assert!(!is_jsonl_request(None));
/// assert!(!is_jsonl_request(Some("application/vnd.api+json")));
/// ```
pub fn is_jsonl_request(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };

    let media_type = content_type.split(';').next().unwrap_or_default().trim();

    media_type.eq_ignore_ascii_case(MEDIA_TYPE)
        || media_type.eq_ignore_ascii_case(MEDIA_TYPE_NDJSON)
}

/// The member carrying an input line's 1-based number on a result line.
///
/// A batch of a hundred thousand lines that reports a failure without saying
/// *which* line failed is not actionable: the client cannot map the error
/// back onto its input. Every error line an ingest emits carries this under
/// the error object's `meta`, so a client can retry exactly the rows that did
/// not land.
///
/// ```text
/// {"errors":[{"status":"422","code":"invalid_attribute","meta":{"line":2}}]}
/// ```
pub const LINE_MEMBER: &str = "line";

/// The shape of a failure that happens *after* the first line is sent.
///
/// A streamed response commits to `200 OK` with its first byte, so a row that
/// fails halfway through cannot change the status — the client has already
/// been told the request succeeded. Truncating silently would leave a partial
/// export indistinguishable from a complete one, which is the worst outcome:
/// the client keeps data it believes is whole.
///
/// So the stream ends with one line carrying this member, and a client must
/// treat its presence as failure however many rows preceded it:
///
/// ```text
/// {"type":"todo","id":"t_1",…}
/// {"errors":[{"status":"500","code":"internal","title":"Internal server error"}]}
/// ```
///
/// This is why the last line has to be checked rather than the status.
pub const ERROR_LINE: &str = "errors";
