//! `?page[…]` and `?sort=` — parsed, checked, and handed to storage.
//!
//! A collection request carries its paging and ordering in the query string.
//! This module turns that string into typed values, rejecting what is
//! malformed before storage is touched, so a `List` implementation receives
//! something it can use rather than raw strings it has to re-parse.
//!
//! Every failure here is a `400` carrying `source.parameter`, which is what
//! JSON:API names for a bad query parameter — the counterpart to the
//! `source.pointer` a bad document member gets.

use std::collections::BTreeMap;

use crate::context::Context;
use crate::error::{ErrorCode, JsonApiError};

/// How many resources a page holds when the client does not say.
pub const DEFAULT_PAGE_SIZE: usize = 25;

/// The largest page a client may ask for.
///
/// A cap is required: without one, `page[limit]=1000000` is a denial of
/// service written in one query parameter.
pub const MAX_PAGE_SIZE: usize = 100;

/// Which page of a collection the client asked for.
///
/// JSON:API leaves pagination strategy to the implementation and only fixes
/// the `page` family of parameters. Both common strategies are supported, and
/// a request may use one or the other — never both:
///
/// - **Offset** — `?page[offset]=20&page[limit]=10`. Gives `first`, `prev`,
///   `next` and, when storage reports a total, `last`.
/// - **Cursor** — `?page[cursor]=t_20&page[size]=10`. Stable while rows are
///   being inserted, and the only sound choice on a large table, at the cost
///   of no `last` link and no page count.
///
/// A [`List`](crate::crud::List) implementation matches on this. Storage that
/// only supports one strategy returns
/// [`ErrorCode::InvalidParameter`] for
/// the other rather than pretending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    /// `page[offset]` / `page[limit]`.
    Offset {
        /// How many resources to skip. Zero on the first page.
        offset: usize,
        /// How many to return, already clamped to [`MAX_PAGE_SIZE`].
        limit: usize,
    },
    /// `page[cursor]` / `page[size]`.
    Cursor {
        /// Where to resume — the value storage handed back last time.
        ///
        /// `None` is the first page, so a client starts by sending
        /// `page[size]` alone.
        after: Option<String>,
        /// How many to return, already clamped to [`MAX_PAGE_SIZE`].
        size: usize,
    },
}

impl Default for Page {
    /// The first page, by offset — what a bare `GET /todos` means.
    fn default() -> Self {
        Self::Offset {
            offset: 0,
            limit: DEFAULT_PAGE_SIZE,
        }
    }
}

impl Page {
    /// How many resources this page holds, whichever strategy is in use.
    pub fn size(&self) -> usize {
        match self {
            Self::Offset { limit, .. } => *limit,
            Self::Cursor { size, .. } => *size,
        }
    }
}

/// One `?sort=` key: a field, and its direction.
///
/// JSON:API spells descending with a leading `-`, so `?sort=-created,message`
/// is "newest first, then by message".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sort {
    field: String,
    descending: bool,
}

impl Sort {
    /// The field to sort on, with the `-` stripped.
    ///
    /// Not checked against the resource's schema: sorting is storage's to
    /// interpret, and only storage knows which columns are indexed or
    /// whether an attribute is even stored. Reject what you do not support
    /// with [`ErrorCode::InvalidParameter`].
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Whether the key was written `-field`.
    pub fn descending(&self) -> bool {
        self.descending
    }
}

/// Everything a collection request asked for.
///
/// Handed to [`List::list`](crate::crud::List::list). Paging is always
/// present — a request that names no page gets the default — and `sort` is
/// empty unless the client sent one.
#[derive(Debug, Clone, Default)]
pub struct Query {
    page: Page,
    sort: Vec<Sort>,
}

impl Query {
    /// The page the client asked for.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// The sort keys, in the order given. Empty when `?sort=` was absent.
    ///
    /// Applying them is storage's business, and so is refusing a field it
    /// cannot sort on — see [`Sort::field`].
    pub fn sort(&self) -> &[Sort] {
        &self.sort
    }

    /// Parse a raw query string.
    ///
    /// Unknown parameters are ignored rather than refused: `?utm_source=…`
    /// on a shared link should not fail the request, and JSON:API reserves
    /// only the families it defines.
    pub fn parse(query: &str, ctx: &Context) -> Result<Self, JsonApiError> {
        let params = split(query);

        Ok(Self {
            page: page_from(&params, ctx)?,
            sort: sort_from(&params),
        })
    }
}

/// `key=value&key=value` → a map, with `%xx` and `+` decoded.
fn split(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => (decode(key), decode(value)),
            None => (decode(pair), String::new()),
        })
        .collect()
}

/// Percent-decoding, plus `+` as a space.
///
/// `page[offset]` arrives as `page%5Boffset%5D` from most clients, so the
/// brackets have to survive the round trip.
fn decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    // Not a valid escape: keep the `%` as written.
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

/// Which paging strategy the parameters describe.
fn page_from(params: &BTreeMap<String, String>, ctx: &Context) -> Result<Page, JsonApiError> {
    let offset = params.get("page[offset]");
    let limit = params.get("page[limit]");
    let cursor = params.get("page[cursor]");
    let size = params.get("page[size]");

    let is_offset = offset.is_some() || limit.is_some();
    let is_cursor = cursor.is_some() || size.is_some();

    // Mixing the two is a client bug worth naming: silently honouring one
    // would page through the collection in a way the caller did not ask for.
    if is_offset && is_cursor {
        return Err(invalid(
            ctx,
            "page",
            "use either `page[offset]`/`page[limit]` or `page[cursor]`/`page[size]`, not both",
        ));
    }

    if is_cursor {
        return Ok(Page::Cursor {
            after: cursor.filter(|value| !value.is_empty()).cloned(),
            size: number(size, "page[size]", DEFAULT_PAGE_SIZE, ctx)?.min(MAX_PAGE_SIZE),
        });
    }

    Ok(Page::Offset {
        offset: number(offset, "page[offset]", 0, ctx)?,
        limit: number(limit, "page[limit]", DEFAULT_PAGE_SIZE, ctx)?.min(MAX_PAGE_SIZE),
    })
}

/// One numeric parameter, or its default.
fn number(
    raw: Option<&String>,
    name: &str,
    default: usize,
    ctx: &Context,
) -> Result<usize, JsonApiError> {
    match raw {
        None => Ok(default),
        Some(value) => value.parse().map_err(|_| {
            invalid(
                ctx,
                name,
                format!("`{name}` must be a non-negative whole number, not `{value}`"),
            )
        }),
    }
}

/// `?sort=-created,message` → the keys, in order.
///
/// Empty keys are dropped, so a trailing comma is not an error.
fn sort_from(params: &BTreeMap<String, String>) -> Vec<Sort> {
    let Some(raw) = params.get("sort") else {
        return Vec::new();
    };

    raw.split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty() && *key != "-")
        .map(|key| match key.strip_prefix('-') {
            Some(field) => Sort {
                field: field.to_string(),
                descending: true,
            },
            None => Sort {
                field: key.to_string(),
                descending: false,
            },
        })
        .collect()
}

/// A `400` naming the parameter at fault.
fn invalid(ctx: &Context, parameter: &str, detail: impl Into<String>) -> JsonApiError {
    JsonApiError::in_request(ErrorCode::InvalidParameter, ctx)
        .detail(detail)
        .parameter(parameter)
}

/// Build a collection's pagination links.
///
/// The URLs a client follows rather than assembling: `self` always, `first`
/// and `prev` when there is somewhere back to go, `next` when there is more,
/// and `last` only when the total is known. Sort keys are carried across
/// every link, so following `next` does not silently reorder the collection.
pub struct Paginator<'a> {
    base: &'a str,
    query: &'a Query,
}

impl<'a> Paginator<'a> {
    /// Links for a collection served at `base`.
    pub fn new(base: &'a str, query: &'a Query) -> Self {
        Self { base, query }
    }

    /// The links for one page.
    ///
    /// `returned` is how many rows this page actually holds, `total` the size
    /// of the collection when storage counted it, `cursor` where the next
    /// page resumes under cursor paging, and `has_more` whether the
    /// collection continues past this page when storage knows exactly.
    pub fn links(
        &self,
        returned: usize,
        total: Option<usize>,
        cursor: Option<&str>,
        has_more: Option<bool>,
    ) -> crate::document::Links {
        match self.query.page() {
            Page::Offset { offset, limit } => {
                self.offset_links(*offset, *limit, returned, total, has_more)
            }
            Page::Cursor { size, .. } => self.cursor_links(*size, cursor),
        }
    }

    /// `first`/`prev`/`next`, and `last` when the total is known.
    fn offset_links(
        &self,
        offset: usize,
        limit: usize,
        returned: usize,
        total: Option<usize>,
        has_more: Option<bool>,
    ) -> crate::document::Links {
        let mut links = crate::document::Links::this(self.offset_url(offset, limit));

        links = links.first(self.offset_url(0, limit));

        if offset > 0 {
            links = links.prev(self.offset_url(offset.saturating_sub(limit), limit));
        }

        // Storage that answered exactly is believed: it read past the page
        // and knows. Otherwise, with a total, "more" is arithmetic; without
        // either, a full page is taken as evidence there may be more — the
        // client discovers the end by following `next` into an empty page,
        // which is the cost of not counting.
        let more = match (has_more, total) {
            (Some(more), _) => more,
            (None, Some(total)) => offset + returned < total,
            (None, None) => returned == limit && limit > 0,
        };

        if more {
            links = links.next(self.offset_url(offset + limit, limit));
        }

        if let Some(total) = total
            && limit > 0
        {
            // The offset of the final page: the largest multiple of `limit`
            // below `total`. An empty collection's last page is its first.
            let last = total.saturating_sub(1) / limit * limit;
            links = links.last(self.offset_url(last, limit));
        }

        links
    }

    /// `self`, and `next` when storage handed back a cursor.
    ///
    /// No `first`, `prev` or `last`: a cursor names one position, and walking
    /// backwards or jumping to the end needs a cursor the server does not
    /// have.
    fn cursor_links(&self, size: usize, cursor: Option<&str>) -> crate::document::Links {
        let current = match self.query.page() {
            Page::Cursor { after, .. } => after.as_deref(),
            Page::Offset { .. } => None,
        };

        let mut links = crate::document::Links::this(self.cursor_url(current, size));

        if let Some(cursor) = cursor {
            links = links.next(self.cursor_url(Some(cursor), size));
        }

        links
    }

    fn offset_url(&self, offset: usize, limit: usize) -> String {
        format!(
            "{}?page[offset]={offset}&page[limit]={limit}{}",
            self.base,
            self.sort_suffix()
        )
    }

    fn cursor_url(&self, after: Option<&str>, size: usize) -> String {
        match after {
            Some(cursor) => format!(
                "{}?page[cursor]={}&page[size]={size}{}",
                self.base,
                encode(cursor),
                self.sort_suffix()
            ),
            None => format!("{}?page[size]={size}{}", self.base, self.sort_suffix()),
        }
    }

    /// `&sort=…`, or nothing when the request carried no sort.
    fn sort_suffix(&self) -> String {
        let keys = self.query.sort();
        if keys.is_empty() {
            return String::new();
        }

        let joined = keys
            .iter()
            .map(|key| {
                if key.descending() {
                    format!("-{}", key.field())
                } else {
                    key.field().to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(",");

        format!("&sort={}", encode(&joined))
    }
}

/// Percent-encode what must survive a round trip through a URL.
///
/// A cursor is storage's opaque string and may hold anything; the sort list
/// keeps its commas and minus signs, which are legal unencoded.
fn encode(raw: &str) -> String {
    raw.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b',' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}
