//! Pure JSON:API document types.
//!
//! These carry no dependency on axum or any data layer, so they work with
//! `default-features = false`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A JSON:API top-level document.
///
/// Exactly one of `data` or `errors` is present in a valid document; the
/// constructors below are the supported way to build one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<PrimaryData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ErrorObject>>,
    /// Compound-document sidecar, populated by `?include=`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<Vec<ResourceObject>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, serde_json::Value>>,
}

impl Document {
    /// A document whose primary data is a single resource.
    pub fn single(resource: ResourceObject) -> Self {
        Self {
            data: Some(PrimaryData::Single(Box::new(resource))),
            ..Self::empty()
        }
    }

    /// A document whose primary data is a collection — a listing response.
    pub fn collection(resources: Vec<ResourceObject>) -> Self {
        Self {
            data: Some(PrimaryData::Collection(resources)),
            ..Self::empty()
        }
    }

    /// An error document. JSON:API allows several errors per response.
    pub fn errors(errors: Vec<ErrorObject>) -> Self {
        Self {
            errors: Some(errors),
            ..Self::empty()
        }
    }

    /// Attach top-level links — pagination links on a listing, for instance.
    pub fn with_links(mut self, links: Links) -> Self {
        self.links = Some(links);
        self
    }

    /// Attach compound-document resources, as requested by `?include=`.
    pub fn with_included(mut self, included: Vec<ResourceObject>) -> Self {
        self.included = Some(included);
        self
    }

    /// Set one top-level `meta` member.
    ///
    /// Creates the `meta` object if the document has none, so members
    /// accumulate rather than replacing one another — a paged listing keeps
    /// its `total` when a `lang` is added beside it.
    #[must_use]
    pub fn with_meta(
        mut self,
        key: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.meta
            .get_or_insert_with(BTreeMap::new)
            .insert(key.into(), value.into());
        self
    }

    fn empty() -> Self {
        Self {
            data: None,
            errors: None,
            included: None,
            links: None,
            meta: None,
        }
    }
}

/// Top-level `data`: one resource, a collection, or an explicit null.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrimaryData {
    Single(Box<ResourceObject>),
    Collection(Vec<ResourceObject>),
}

/// A JSON:API resource object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceObject {
    /// The resource type — `ResourceSchema::name`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Absent on a client-generated create, present everywhere else.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// A redacted or unrequested attribute is *absent*, never `null`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub attributes: BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub relationships: BTreeMap<String, Relationship>,
    /// Aggregates land here — `ResourceSchema::aggregates`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,
}

impl ResourceObject {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: Some(id.into()),
            attributes: BTreeMap::new(),
            relationships: BTreeMap::new(),
            meta: None,
            links: None,
        }
    }

    pub fn attr(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub fn relationship(mut self, name: impl Into<String>, rel: Relationship) -> Self {
        self.relationships.insert(name.into(), rel);
        self
    }

    pub fn with_links(mut self, links: Links) -> Self {
        self.links = Some(links);
        self
    }
}

/// A relationship: linkage plus its `self`/`related` links.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Linkage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,
}

impl Relationship {
    /// A to-one relationship — `todo` belongs_to `user` as `owner`.
    pub fn to_one(identifier: ResourceIdentifier) -> Self {
        Self {
            data: Some(Linkage::One(Some(identifier))),
            links: None,
        }
    }

    /// A to-many relationship — `user` has_many `todo`.
    pub fn to_many(identifiers: Vec<ResourceIdentifier>) -> Self {
        Self {
            data: Some(Linkage::Many(identifiers)),
            links: None,
        }
    }

    /// An empty to-one relationship — an explicit `null`, distinct from absent.
    pub fn empty_to_one() -> Self {
        Self {
            data: Some(Linkage::One(None)),
            links: None,
        }
    }

    pub fn with_links(mut self, links: Links) -> Self {
        self.links = Some(links);
        self
    }
}

/// Relationship linkage. `One(None)` serializes as `null`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Linkage {
    One(Option<ResourceIdentifier>),
    Many(Vec<ResourceIdentifier>),
}

/// A `{ type, id }` pair.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResourceIdentifier {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
}

impl ResourceIdentifier {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }
}

/// A links object. Only the members JSON:API defines for our responses.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Links {
    #[serde(rename = "self", skip_serializing_if = "Option::is_none")]
    pub this: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
}

impl Links {
    pub fn this(url: impl Into<String>) -> Self {
        Self {
            this: Some(url.into()),
            ..Self::default()
        }
    }

    pub fn related(mut self, url: impl Into<String>) -> Self {
        self.related = Some(url.into());
        self
    }

    pub fn first(mut self, url: impl Into<String>) -> Self {
        self.first = Some(url.into());
        self
    }

    pub fn next(mut self, url: impl Into<String>) -> Self {
        self.next = Some(url.into());
        self
    }

    pub fn prev(mut self, url: impl Into<String>) -> Self {
        self.prev = Some(url.into());
        self
    }

    pub fn last(mut self, url: impl Into<String>) -> Self {
        self.last = Some(url.into());
        self
    }
}

/// A JSON:API error object.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ErrorObject {
    /// The correlation id from the current `ash-log` scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub status: String,
    pub code: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ErrorSource>,
    /// Non-standard members, which JSON:API confines to `meta`.
    ///
    /// Carries `i18n` — a [`Translation`](crate::i18n::Translation) key and
    /// its params — when the error was given one, so a client can render the
    /// failure in its own language instead of matching on English `detail`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, serde_json::Value>>,
}

/// Points at whatever caused the error — a body member or a query parameter.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ErrorSource {
    /// A JSON pointer, e.g. `/data/attributes/message`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    /// A query parameter, e.g. `sort`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
}

impl ErrorSource {
    pub fn pointer(p: impl Into<String>) -> Self {
        Self {
            pointer: Some(p.into()),
            parameter: None,
        }
    }

    pub fn parameter(p: impl Into<String>) -> Self {
        Self {
            pointer: None,
            parameter: Some(p.into()),
        }
    }
}

/// The JSON:API media type. Every request and response carries it.
pub const MEDIA_TYPE: &str = "application/vnd.api+json";
