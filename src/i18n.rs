//! Translation keys on errors, and the negotiated locale on the response.
//!
//! An error's `title` and `detail` are English prose. That is fine for a log
//! and useless for a user interface that has to render the same failure in
//! whatever language its user reads, so a client is left matching on English
//! substrings — which breaks the moment the wording is improved.
//!
//! This crate does not translate. It emits a **stable key** and the **params**
//! the message interpolates, and the client renders them against its own
//! catalog:
//!
//! ```json
//! {
//!   "status": "422",
//!   "code": "invalid_attribute",
//!   "title": "Invalid attribute",
//!   "detail": "must be 80 characters or fewer",
//!   "source": { "pointer": "/data/attributes/name" },
//!   "meta": {
//!     "i18n": {
//!       "key": "todo.name.too_long",
//!       "params": { "max": 80 }
//!     }
//!   }
//! }
//! ```
//!
//! `title` and `detail` stay exactly as they were — they are the fallback for
//! a client with no catalog, and dropping them would break every existing
//! consumer. The key is *additional*, which is also why it lives under `meta`:
//! JSON:API fixes the members an error object may carry, and `meta` is the
//! member it provides for everything else. A client that does not know about
//! it sees an ordinary error object.
//!
//! # Why the server does not translate
//!
//! Reading `Accept-Language` and returning German prose is the other way to
//! do this, and it is worse here: it puts a message catalog, a negotiation
//! table and a release cycle for every wording change inside the API server,
//! and it still cannot localize what the client composes around the message.
//! The client already has a catalog — it renders its own UI — so the useful
//! thing to send it is the key, not a second translation of it.
//!
//! What the server *does* do is record which locale the client asked for, on
//! the document's top-level `meta`, so a response is self-describing about
//! what it negotiated. See [`Locale`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A translation key and the params its message interpolates.
///
/// Attached to a [`JsonApiError`](crate::JsonApiError) with
/// [`translate`](crate::JsonApiError::translate), and serialized under the
/// error object's `meta.i18n`.
///
/// The key is yours: this crate never invents one, and never rewrites one.
/// Keep it stable across wording changes — that is the whole point of it —
/// and namespace it however your catalog is organized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Translation {
    /// The catalog key, e.g. `todo.name.too_long`.
    pub key: String,
    /// The values the message interpolates, e.g. `{ "max": 80 }`.
    ///
    /// Omitted from the response when empty, so a message with no
    /// placeholders does not carry an empty object.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
}

impl Translation {
    /// A key with no params — a message with nothing to interpolate.
    ///
    /// ```
    /// use ash_jsonapi::i18n::Translation;
    ///
    /// let t = Translation::new("todo.not_found");
    /// assert_eq!(t.key, "todo.not_found");
    /// assert!(t.params.is_empty());
    /// ```
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            params: BTreeMap::new(),
        }
    }

    /// Add one interpolation param.
    ///
    /// ```
    /// use ash_jsonapi::i18n::Translation;
    ///
    /// let t = Translation::new("todo.name.too_long").param("max", 80);
    /// assert_eq!(t.params["max"], 80);
    /// ```
    #[must_use]
    pub fn param(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }
}

/// The locale the client asked for, as negotiated from `Accept-Language`.
///
/// Recorded on the response's top-level `meta.lang` so a response says which
/// locale it was produced for, and carried on the [`Context`](crate::Context)
/// so a handler that *does* have a catalog can consult it.
///
/// This is the client's request, not a promise: nothing in this crate
/// translates, so `lang` describes what was asked for and the
/// [`Translation`] keys are what make honouring it possible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locale(String);

impl Locale {
    /// The best match from an `Accept-Language` header value.
    ///
    /// Ranked by the `q` values the header carries, highest first, with a
    /// missing `q` meaning `q=1` as [RFC 9110 §12.4.2] specifies. `*` is
    /// skipped — it names no locale to record — and so is `q=0`, which is a
    /// client saying it will *not* accept that language.
    ///
    /// Returns `None` when the header is absent, empty, or names nothing
    /// usable; the response then carries no `lang` rather than a guess.
    ///
    /// ```
    /// use ash_jsonapi::i18n::Locale;
    ///
    /// let best = Locale::negotiate("de-DE,de;q=0.9,en;q=0.8").unwrap();
    /// assert_eq!(best.as_str(), "de-DE");
    ///
    /// // `q` ordering wins over position:
    /// let best = Locale::negotiate("en;q=0.5, fr;q=0.9").unwrap();
    /// assert_eq!(best.as_str(), "fr");
    ///
    /// assert!(Locale::negotiate("*").is_none());
    /// ```
    ///
    /// [RFC 9110 §12.4.2]: https://www.rfc-editor.org/rfc/rfc9110#section-12.4.2
    pub fn negotiate(header: &str) -> Option<Self> {
        let mut best: Option<(f32, usize, &str)> = None;

        for (position, entry) in header.split(',').enumerate() {
            let mut parts = entry.split(';');

            let tag = parts.next()?.trim();
            if tag.is_empty() || tag == "*" || !is_language_tag(tag) {
                continue;
            }

            // `;q=` is the only parameter that concerns us; anything else on
            // the entry is ignored rather than treated as a parse failure.
            let quality = parts
                .find_map(|part| part.trim().strip_prefix("q=")?.trim().parse::<f32>().ok())
                .unwrap_or(1.0);

            // `q=0` means "not acceptable", so it never wins.
            if quality <= 0.0 {
                continue;
            }

            // Strictly greater, so an equal `q` keeps the earlier entry —
            // header order is the tie-break the RFC leaves to the server.
            let better = match best {
                Some((best_q, _, _)) => quality > best_q,
                None => true,
            };
            if better {
                best = Some((quality, position, tag));
            }
        }

        best.map(|(_, _, tag)| Self(tag.to_string()))
    }

    /// The language tag, e.g. `de-DE`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Locale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether a token is shaped like a language tag.
///
/// Deliberately strict: this value is echoed into a response body, so it is
/// restricted to what a tag may contain rather than reflected verbatim.
/// Length-capped for the same reason — a header is client-supplied input.
fn is_language_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 35 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
