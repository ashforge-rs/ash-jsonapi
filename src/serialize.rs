//! Your data + your schema → [`ResourceObject`].
//!
//! Describe a resource once by implementing [`Schema`] and
//! [`RelationshipDef`], and this module writes the JSON:API envelope for it:
//! the `type`/`id` wrapper, relationship linkage, and `self`/`related` links.
//! Nothing here needs a particular ORM or data layer — a `Schema` is three
//! methods over whatever types you already have.
//!
//! ```ignore
//! struct Todo;
//!
//! impl Schema for Todo {
//!     type Relationship = Rel;
//!
//!     fn name(&self) -> &str { "todo" }
//!     fn base_path(&self) -> &str { "/todos" }
//!     fn relationships(&self) -> &[Rel] { &[Rel::to_one("owner", "user")] }
//! }
//!
//! let object = Serializer::new(&Todo)
//!     .record("t_1")
//!     .attr("message", "buy milk")
//!     .rel("owner", "u_1")
//!     .build();
//! ```
//!
//! The point is that you state a relationship's target type and arity *once*,
//! in the schema. The linkage shape and both link URLs follow from it, so no
//! call site spells them and none can drift.
//!
//! Using `ash-domain`? Its `ResourceSchema` implements these traits already —
//! see [`crate::domain`], and skip writing them yourself.

use std::collections::BTreeMap;

use crate::document::{Linkage, Links, Relationship, ResourceIdentifier, ResourceObject};

/// One relationship your resource declares.
///
/// Only the arity matters to JSON:API: a to-one relationship serializes as a
/// single linkage object, a to-many as an array. Whatever distinction your
/// data layer draws beyond that — belongs-to versus has-one, join tables — is
/// its business, not the wire format's.
///
/// [`Rel`] is a ready-made implementation; write your own only if you already
/// have a relationship type to borrow.
pub trait RelationshipDef {
    /// The member name — `owner`, `comments`.
    fn name(&self) -> &str;

    /// The JSON:API `type` on the far side: the related resource's name.
    fn target(&self) -> &str;

    /// Whether the far side holds many resources.
    fn is_to_many(&self) -> bool;
}

/// A relationship, ready to use.
///
/// Saves declaring a type just to satisfy [`Schema::Relationship`]:
///
/// ```ignore
/// fn relationships(&self) -> &[Rel] {
///     &[Rel::to_one("owner", "user"), Rel::to_many("tags", "tag")]
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rel {
    name: &'static str,
    target: &'static str,
    to_many: bool,
}

impl Rel {
    /// A relationship with one resource on the far side.
    pub const fn to_one(name: &'static str, target: &'static str) -> Self {
        Self {
            name,
            target,
            to_many: false,
        }
    }

    /// A relationship with many resources on the far side.
    pub const fn to_many(name: &'static str, target: &'static str) -> Self {
        Self {
            name,
            target,
            to_many: true,
        }
    }
}

impl RelationshipDef for Rel {
    fn name(&self) -> &str {
        self.name
    }

    fn target(&self) -> &str {
        self.target
    }

    fn is_to_many(&self) -> bool {
        self.to_many
    }
}

/// A resource with no relationships.
///
/// The `Schema::Relationship` to use when there are none — so a flat resource
/// needs no relationship type of its own:
///
/// ```ignore
/// impl Schema for Note {
///     type Relationship = NoRelationships;
///
///     fn name(&self) -> &str { "note" }
///     fn base_path(&self) -> &str { "/notes" }
///     fn relationships(&self) -> &[NoRelationships] { &[] }
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoRelationships {}

impl RelationshipDef for NoRelationships {
    fn name(&self) -> &str {
        match *self {}
    }

    fn target(&self) -> &str {
        match *self {}
    }

    fn is_to_many(&self) -> bool {
        match *self {}
    }
}

/// What serialization needs to know about one resource.
///
/// Three methods. Implement it on whatever already describes your resource —
/// a marker struct, a config value, or a schema type you own.
pub trait Schema {
    type Relationship: RelationshipDef;

    /// `ResourceSchema::name` → the JSON:API `type`.
    fn name(&self) -> &str;

    /// Every declared relationship.
    fn relationships(&self) -> &[Self::Relationship];

    /// Where the resource is mounted, e.g. `/api/v1/todos`.
    ///
    /// Used to build `self` and `related` links, so it should match the path
    /// the router actually serves.
    fn base_path(&self) -> &str;
}

/// Serializes one resource's records into JSON:API resource objects.
///
/// Borrows the schema rather than copying it, so your declaration stays the
/// single source of truth.
#[derive(Debug, Clone, Copy)]
pub struct Serializer<'a, S: Schema> {
    schema: &'a S,
}

impl<'a, S: Schema> Serializer<'a, S> {
    pub fn new(schema: &'a S) -> Self {
        Self { schema }
    }

    /// The `self` link for one resource.
    pub fn self_link(&self, id: &str) -> String {
        format!("{}/{}", self.schema.base_path().trim_end_matches('/'), id)
    }

    /// Start building a resource object for `id`.
    ///
    /// The usual entry point. Attributes and relationships are added
    /// fluently, and the type, the `self` link and every relationship link
    /// come from the schema:
    ///
    /// ```text
    /// serializer
    ///     .record(&id)
    ///     .attr("message", "buy milk")
    ///     .rel("owner", "u_01H8X")
    ///     .build()
    /// ```ignore
    pub fn record(&self, id: &str) -> Record<'a, S> {
        self.record_owned(id.to_string())
    }

    /// [`record`](Self::record), taking an id you already own.
    ///
    /// The same builder, without copying the id. A caller that has just
    /// rendered its primary key — which [`resource!`](crate::resource!) does
    /// on every row — would otherwise allocate it, lend it here, and have it
    /// allocated again immediately.
    pub fn record_owned(&self, id: String) -> Record<'a, S> {
        Record {
            schema: self.schema,
            id,
            attributes: BTreeMap::new(),
            related: BTreeMap::new(),
        }
    }

    /// Serialize a record from prepared maps.
    ///
    /// `attributes` is the record's data, already projected for any sparse
    /// fieldset. `related` maps a relationship name to the ids on its far
    /// side. Prefer [`Serializer::record`] unless the maps already exist —
    /// this is the bulk path, for a listing built from a data layer.
    pub fn serialize(
        &self,
        id: &str,
        attributes: BTreeMap<String, serde_json::Value>,
        related: &BTreeMap<String, Vec<String>>,
    ) -> ResourceObject {
        // The row's `self` link is the prefix every one of its relationship
        // links is built from, so it is computed once here and borrowed by
        // each. Rebuilding it per relationship was the single most expensive
        // thing a listing did: a row with three relationships formatted the
        // same `{base}/{id}` four times, and a hundred-row page did it four
        // hundred.
        let self_link = self.self_link(id);

        let mut object = ResourceObject::new(self.schema.name(), id);
        object.attributes = attributes;

        // Borrowed while the relationships are built, then moved onto the
        // object — so a flat resource, which has no relationships to share
        // the prefix with, never copies it.
        for (name, ids) in related {
            if let Some(rel) = self.relationship_at(&self_link, name, ids) {
                object = object.relationship(name.clone(), rel);
            }
        }

        object.with_links(Links::this(self_link))
    }

    /// Build one relationship's linkage and links from the schema.
    ///
    /// Returns `None` for a name the schema does not declare.
    pub fn relationship(&self, id: &str, name: &str, ids: &[String]) -> Option<Relationship> {
        self.relationship_at(&self.self_link(id), name, ids)
    }

    /// [`relationship`](Self::relationship), against an already-built `self`
    /// link.
    ///
    /// Split out so a row that carries several relationships formats its own
    /// link once rather than once per relationship — see [`Self::serialize`].
    fn relationship_at(&self, self_link: &str, name: &str, ids: &[String]) -> Option<Relationship> {
        self.relationship_owned(self_link, name, ids.to_vec())
    }

    /// [`relationship_at`](Self::relationship_at), taking ids it may consume.
    ///
    /// The builder owns its linkage ids and drops them straight after, so
    /// moving them onto the identifiers saves a copy of every id on every
    /// relationship of every row.
    fn relationship_owned(
        &self,
        self_link: &str,
        name: &str,
        ids: Vec<String>,
    ) -> Option<Relationship> {
        let def = self
            .schema
            .relationships()
            .iter()
            .find(|r| r.name() == name)?;

        let identifiers: Vec<_> = ids
            .into_iter()
            .map(|target_id| ResourceIdentifier::new(def.target(), target_id))
            .collect();

        // Arity is the schema's, not the data's: a to-one relationship with no
        // target is an explicit null, never an empty array.
        let data = if def.is_to_many() {
            Linkage::Many(identifiers)
        } else {
            Linkage::One(identifiers.into_iter().next())
        };

        // Both links extend the same prefix, so each is one allocation sized
        // up front rather than a `format!` that measures and grows.
        let mut this =
            String::with_capacity(self_link.len() + "/relationships/".len() + name.len());
        this.push_str(self_link);
        this.push_str("/relationships/");
        this.push_str(name);

        let mut related = String::with_capacity(self_link.len() + 1 + name.len());
        related.push_str(self_link);
        related.push('/');
        related.push_str(name);

        Some(Relationship {
            data: Some(data),
            links: Some(Links::this(this).related(related)),
        })
    }
}

/// A resource object under construction. Created by [`Serializer::record`].
#[must_use = "a record does nothing until `build` is called"]
pub struct Record<'a, S: Schema> {
    schema: &'a S,
    id: String,
    attributes: BTreeMap<String, serde_json::Value>,
    related: BTreeMap<String, Vec<String>>,
}

impl<S: Schema> Record<'_, S> {
    /// Set one attribute.
    pub fn attr(mut self, name: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    /// Set every attribute at once, from a record's own data.
    pub fn attrs(
        mut self,
        attributes: impl IntoIterator<Item = (String, serde_json::Value)>,
    ) -> Self {
        self.attributes.extend(attributes);
        self
    }

    /// Link a to-one relationship. The target type comes from the schema.
    pub fn rel(self, name: impl Into<String>, id: impl Into<String>) -> Self {
        self.rel_many(name, [id.into()])
    }

    /// Link a to-many relationship.
    pub fn rel_many(
        mut self,
        name: impl Into<String>,
        ids: impl IntoIterator<Item = String>,
    ) -> Self {
        self.related.insert(name.into(), ids.into_iter().collect());
        self
    }

    /// Finish the resource object.
    ///
    /// Consumes the builder's own maps rather than handing
    /// [`serialize`](Serializer::serialize) a reference to them: the linkage
    /// ids are moved onto the resource object instead of being cloned out of
    /// a map that is about to be dropped.
    pub fn build(self) -> ResourceObject {
        let serializer = Serializer::new(self.schema);
        let self_link = serializer.self_link(&self.id);

        let mut object = ResourceObject::new(self.schema.name(), self.id);
        object.attributes = self.attributes;

        for (name, ids) in self.related {
            if let Some(rel) = serializer.relationship_owned(&self_link, &name, ids) {
                object = object.relationship(name, rel);
            }
        }

        object.with_links(Links::this(self_link))
    }
}
