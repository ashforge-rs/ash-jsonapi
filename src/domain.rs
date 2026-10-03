//! Where `ash-domain` plugs in — **optional**, behind the `domain` feature.
//!
//! This crate stands alone: [`resource!`](crate::resource!) over your own
//! structs and storage needs no `ash-*` crate in the tree at all. This module
//! is the other way in, for a service whose resources the domain already
//! owns.
//!
//! The division: the domain owns resources, policies and execution; this
//! crate owns the JSON:API wire format. Nothing about a resource is declared
//! twice — attributes, types, relationships and cardinality all come from
//! `#[derive(Resource)]`, and the one thing supplied here is the base path,
//! because the domain does not know it is being served over HTTP.
//!
//! ```text
//! impl DomainState for App { /* domain, context, mounted schemas */ }
//!
//! ash_jsonapi::domain_resource! { Todo as "todo" at "/todos", state: App }
//! ash_jsonapi::api! { operations: [ash_jsonapi::CRUD], resource: Todo, … }
//! ```
//!
//! [`domain_resource!`](crate::domain_resource!) writes the same traits
//! [`resource!`](crate::resource!) does, so [`api!`](crate::api!) serves both
//! kinds of resource identically. Every action runs *through* the domain —
//! [`Bound`](ash_domain::domain::Bound), not around it — so policies,
//! extensions and validation all apply, and [`error_from`] turns what comes
//! back into the right status: a denial is a `403`, not a `500`.

use ash_domain::schema::{RelationshipSchema, ResourceSchema, SchemaType};
use ash_domain::value::{Record as DomainRecord, Value};

use crate::context::Context;
use crate::error::JsonApiError;
use crate::serialize::{Record, RelationshipDef, Schema, Serializer};

impl RelationshipDef for RelationshipSchema {
    fn name(&self) -> &str {
        &self.name
    }

    fn target(&self) -> &str {
        &self.destination
    }

    fn is_to_many(&self) -> bool {
        // `RelationshipSchema` reports cardinality as a string rather than the
        // `Cardinality` enum, so match the strings it documents.
        matches!(self.cardinality, "has_many" | "many_to_many")
    }
}

/// A resource schema paired with the path it is served under.
///
/// The base path is not a domain concept — the domain does not know it is
/// being served over HTTP — so the router supplies it alongside the schema.
#[derive(Debug, Clone, Copy)]
pub struct Mounted<'a> {
    pub schema: &'a ResourceSchema,
    pub base_path: &'a str,
}

impl<'a> Mounted<'a> {
    pub fn new(schema: &'a ResourceSchema, base_path: &'a str) -> Self {
        Self { schema, base_path }
    }

    /// Start a resource object for `id`, in one step.
    ///
    /// ```text
    /// Mounted::new(&schema, "/api/v1/todos")
    ///     .record("t_1")
    ///     .attr("message", "buy milk")
    ///     .rel("owner", "u_01H8X")
    ///     .build()
    /// ```ignore
    pub fn record(&self, id: &str) -> Record<'_, Self> {
        Serializer::new(self).record(id)
    }

    /// The `self` link for one resource.
    pub fn self_link(&self, id: &str) -> String {
        Serializer::new(self).self_link(id)
    }

    /// Serialize an `ash_domain::Record` straight from the domain.
    ///
    /// Two things are dropped from `attributes`, because JSON:API carries
    /// them elsewhere: the primary key, which becomes the envelope `id`, and
    /// every relationship's source attribute — a foreign key belongs in
    /// linkage, not beside the data. Everything else is converted as-is.
    ///
    /// Relationships are added afterwards, since a `Record` holds foreign
    /// keys rather than linkage:
    ///
    /// ```ignore
    /// mounted.record_from(&record).rel("owner", owner_id).build()
    /// ```
    pub fn record_from(&self, record: &DomainRecord) -> Record<'_, Self> {
        // The primary key becomes the envelope `id`, which is always a string.
        let id = match record.get(&self.schema.primary_key) {
            Some(Value::Str(s)) => s.clone(),
            Some(other) => render(other).to_string(),
            None => String::new(),
        };

        let hidden = |key: &String| {
            key == &self.schema.primary_key
                || self
                    .schema
                    .relationships
                    .iter()
                    .any(|rel| &rel.source_attribute == key)
        };

        let attributes = record
            .0
            .iter()
            .filter(|(key, _)| !hidden(key))
            .map(|(key, value)| (key.clone(), render(value)));

        self.record(&id).attrs(attributes)
    }
}

/// `ash_domain::Value` → `serde_json::Value`.
///
/// Written out rather than delegated to `serde_json::to_value`: `Value`'s own
/// `Serialize` is externally tagged, so a message would reach the client as
/// `{"Str":"buy milk"}`. A JSON:API attribute is the bare value.
fn render(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => (*b).into(),
        Value::Int(i) => (*i).into(),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Str(s) => s.clone().into(),
        // Bytes have no JSON form; base64 is the caller's decision, so a
        // resource that exposes them should use a computed attribute.
        Value::Bytes(_) => serde_json::Value::Null,
        // Milliseconds since the epoch, as the domain stores them.
        Value::Timestamp(ms) => (*ms).into(),
        Value::Map(map) => {
            serde_json::Value::Object(map.iter().map(|(k, v)| (k.clone(), render(v))).collect())
        }
        Value::List(items) => serde_json::Value::Array(items.iter().map(render).collect()),
    }
}

impl Schema for Mounted<'_> {
    type Relationship = RelationshipSchema;

    fn name(&self) -> &str {
        &self.schema.name
    }

    fn relationships(&self) -> &[RelationshipSchema] {
        &self.schema.relationships
    }

    fn base_path(&self) -> &str {
        self.base_path
    }
}

/// Build an [`ash_domain::schema::ResourceSchema`] literal.
///
/// `ResourceSchema` has ten fields, most of which serialization ignores.
/// Writing one by hand is noise; this fills in the defaults.
///
/// **For tests, examples and fixtures only.** A real service takes its schema
/// from `Domain::schema()` — a resource is declared once, with
/// `#[derive(Resource)]`, and never restated.
///
/// ```ignore
/// use ash_jsonapi::resource_schema;
///
/// let schema = resource_schema! {
///     "todo" {
///         message: String,
///         owner -> "user",
///         tags -> ["tag"],
///     }
/// };
///
/// assert_eq!(schema.name, "todo");
/// assert_eq!(schema.attributes.len(), 1);
/// assert_eq!(schema.relationships.len(), 2);
/// ```
///
/// `name: Type` declares an attribute — `String`, `Integer`, `Number`,
/// `Boolean` or `Bytes`. `-> "user"` declares a to-one relationship;
/// `-> ["tag"]` a to-many, the brackets reading as "many of these".
#[macro_export]
macro_rules! resource_schema {
    ($name:literal $({ $($member:tt)* })?) => {{
        #[allow(unused_mut)]
        let mut attributes: ::std::vec::Vec<
            $crate::__ash_domain::schema::AttributeSchema,
        > = ::std::vec::Vec::new();
        #[allow(unused_mut)]
        let mut relationships: ::std::vec::Vec<
            $crate::__ash_domain::schema::RelationshipSchema,
        > = ::std::vec::Vec::new();
        $($crate::__schema_members!(attributes, relationships, $($member)*);)?

        $crate::__ash_domain::schema::ResourceSchema {
            name: ::std::string::String::from($name),
            storage_name: ::std::string::String::from($name),
            primary_key: ::std::string::String::from("id"),
            version_attribute: ::std::option::Option::None,
            tenant: ::std::option::Option::None,
            attributes,
            relationships,
            aggregates: ::std::vec::Vec::new(),
            computed: ::std::vec::Vec::new(),
            actions: ::std::vec::Vec::new(),
        }
    }};
}

/// Walks the member list, sorting attributes from relationships.
#[doc(hidden)]
#[macro_export]
macro_rules! __schema_members {
    ($attrs:ident, $rels:ident $(,)?) => {};

    ($attrs:ident, $rels:ident, $name:ident : $ty:ident $(, $($rest:tt)*)?) => {
        $attrs.extend([$crate::__ash_domain::schema::AttributeSchema {
            name: ::std::string::String::from(::std::stringify!($name)),
            ty: $crate::__ash_domain::schema::SchemaType::$ty,
            default: ::std::option::Option::None,
            primary_key: false,
            attribute_policy: false,
        }]);
        $($crate::__schema_members!($attrs, $rels, $($rest)*);)?
    };

    ($attrs:ident, $rels:ident, $name:ident -> $target:tt $(, $($rest:tt)*)?) => {
        $rels.extend([$crate::__relationship_schema!($name -> $target)]);
        $($crate::__schema_members!($attrs, $rels, $($rest)*);)?
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __relationship_schema {
    ($name:ident -> [$target:literal]) => {
        $crate::__relationship_schema!(@build $name, $target, "has_many")
    };
    ($name:ident -> $target:literal) => {
        $crate::__relationship_schema!(@build $name, $target, "belongs_to")
    };
    (@build $name:ident, $target:literal, $cardinality:literal) => {
        $crate::__ash_domain::schema::RelationshipSchema {
            name: ::std::string::String::from(::std::stringify!($name)),
            destination: ::std::string::String::from($target),
            cardinality: $cardinality,
            source_attribute: ::std::string::String::from("id"),
            destination_attribute: ::std::string::String::from("id"),
            through: ::std::option::Option::None,
        }
    };
}

impl<'a> Mounted<'a> {
    /// Read a create document into the resource's own type.
    ///
    /// The document is expected to have passed
    /// [`DocumentValidator`](crate::validation::DocumentValidator) already —
    /// shape, required members and linkage are its job. What remains here is
    /// the typed conversion, driven by the schema's declared attribute types.
    pub fn parse_create<R>(
        &self,
        document: &serde_json::Value,
        ctx: &Context,
    ) -> Result<R, JsonApiError>
    where
        R: ash_domain::FromRecord + ash_domain::value::IntoRecord + Default,
    {
        self.parse_document(document, None, ctx)
    }

    /// Read an update document into the resource's own type.
    ///
    /// Same as [`Mounted::parse_create`], except the primary key comes from
    /// the URL: a `PATCH` addresses a row that already exists, and the id in
    /// the path is authoritative over anything the body says.
    pub fn parse_update<R>(
        &self,
        document: &serde_json::Value,
        id: &str,
        ctx: &Context,
    ) -> Result<R, JsonApiError>
    where
        R: ash_domain::FromRecord + ash_domain::value::IntoRecord + Default,
    {
        self.parse_document(document, Some(id), ctx)
    }

    fn parse_document<R>(
        &self,
        document: &serde_json::Value,
        id: Option<&str>,
        ctx: &Context,
    ) -> Result<R, JsonApiError>
    where
        R: ash_domain::FromRecord + ash_domain::value::IntoRecord + Default,
    {
        // Start from the resource's own `Default`, so any attribute the body
        // omits keeps the value the struct would have given it. That is the
        // only source that knows a valid `ValueEnum` member — a schema
        // records the type, not which variant is the default.
        let mut record = R::default()
            .into_record()
            .map_err(|err| JsonApiError::internal(ctx).detail(err.to_string()))?;

        let supplied = self.read_document(document, ctx)?;
        record.0.extend(supplied.0);

        // On an update the row already exists, so the path's id wins; on a
        // create `Default` leaves it empty and the domain stamps one.
        if let Some(id) = id {
            record
                .0
                .insert(self.schema.primary_key.clone(), Value::Str(id.to_string()));
        }

        R::from_record(&record).map_err(|err| {
            // The schema and the struct disagree, which is a bug in the
            // resource declaration rather than in the request.
            JsonApiError::invalid(ctx, err.to_string(), "/data/attributes")
        })
    }
}

impl Mounted<'_> {
    /// `{"data":{"attributes":…,"relationships":…}}` → `Record`.
    ///
    /// Attributes are coerced to the types the schema declares; relationship
    /// linkage is written back to the foreign key it stands for.
    fn read_document(
        &self,
        document: &serde_json::Value,
        ctx: &Context,
    ) -> Result<DomainRecord, JsonApiError> {
        let invalid =
            |detail: String, pointer: &str| JsonApiError::invalid(ctx, detail, pointer.to_string());

        let data = document
            .get("data")
            .ok_or_else(|| invalid("`data` is required".into(), "/data"))?;

        let mut record = DomainRecord::new();

        // Attributes, coerced to the types the schema declares. An attribute
        // the schema does not know is ignored rather than passed through:
        // the domain would reject it, and a clear message beats a 500.
        //
        // The primary key is skipped for the same reason `record_from` drops
        // it on the way out: JSON:API carries it in the envelope, not among
        // the attributes. `DocumentValidator::for_create` hides it from the
        // *request schema*, but hiding a property only stops describing it —
        // nothing sets `additionalProperties: false` — so without this the
        // member would still be read, and a client could choose its own row
        // id by sending `attributes.id`. A create lets the domain stamp the
        // key; an update takes it from the path, which `parse_document` and
        // `record_for_update` apply after this returns.
        if let Some(attributes) = data.get("attributes").and_then(|a| a.as_object()) {
            for attribute in &self.schema.attributes {
                if attribute.name == self.schema.primary_key {
                    continue;
                }

                let value = match attributes.get(&attribute.name) {
                    Some(json) => coerce(json, &attribute.ty).ok_or_else(|| {
                        invalid(
                            format!("`{}` is not of the declared type", attribute.name),
                            &format!("/data/attributes/{}", attribute.name),
                        )
                    })?,
                    // Absent from the body: leave whatever the resource's
                    // own `Default` put there.
                    None => continue,
                };

                record.0.insert(attribute.name.clone(), value);
            }
        }

        // Linkage → foreign keys. A relationship's `source_attribute` is the
        // column the domain stores, so writing it here is what makes
        // `{"owner":{"data":{"id":"alice"}}}` mean `owner_id = "alice"`.
        if let Some(relationships) = data.get("relationships").and_then(|r| r.as_object()) {
            for rel in &self.schema.relationships {
                let Some(id) = relationships
                    .get(&rel.name)
                    .and_then(|r| r.get("data"))
                    .and_then(|d| d.get("id"))
                    .and_then(|id| id.as_str())
                else {
                    continue;
                };

                record
                    .0
                    .insert(rel.source_attribute.clone(), Value::Str(id.to_string()));
            }
        }

        Ok(record)
    }
}

/// `serde_json::Value` → `ash_domain::Value`, against a declared type.
///
/// Returns `None` when the JSON does not fit, which becomes a 422 pointing at
/// the attribute.
fn coerce(json: &serde_json::Value, ty: &SchemaType) -> Option<Value> {
    if json.is_null() {
        return Some(Value::Null);
    }

    match ty {
        SchemaType::Boolean => json.as_bool().map(Value::Bool),
        SchemaType::Integer => json.as_i64().map(Value::Int),
        SchemaType::Number => json.as_f64().map(Value::Float),
        SchemaType::String => json.as_str().map(|s| Value::Str(s.to_string())),
        // A reference is stored as the target's id.
        SchemaType::Reference { .. } => json.as_str().map(|s| Value::Str(s.to_string())),
        // `ValueEnum` and other consumer scalars reach storage as strings.
        SchemaType::Opaque { .. } => json.as_str().map(|s| Value::Str(s.to_string())),
        SchemaType::Embed { .. } | SchemaType::EmbedList { .. } => {
            // Nested structures round-trip through the same JSON form the
            // domain stores them in.
            serde_json::from_value(json.clone()).ok()
        }
        // Bytes have no JSON form; a resource exposing them needs an explicit
        // encoding, which is the resource's decision rather than ours.
        SchemaType::Bytes => None,
    }
}

#[cfg(feature = "validation")]
#[cfg_attr(docsrs, doc(cfg(feature = "validation")))]
impl crate::validation::DocumentValidator {
    /// Derive a create-body validator from an `ash-domain` schema.
    ///
    /// The ash-domain shortcut for
    /// [`DocumentValidator::builder`](crate::validation::DocumentValidator::builder):
    /// attribute types come from `ResourceSchema::to_json_schema`, and the
    /// members JSON:API carries elsewhere are hidden — the primary key, which
    /// belongs in the envelope, and every relationship's foreign key, which
    /// belongs in linkage.
    ///
    /// Note what hiding does and does not do. [`hide`](crate::validation::ValidatorBuilder::hide)
    /// removes a property from the generated schema, so the member is no
    /// longer *described*; it does not set `additionalProperties: false`, so
    /// it is not *forbidden* either. Sending a hidden member is therefore not
    /// an error — it is ignored. What keeps it from taking effect is the
    /// reading side: `Mounted`'s document reader skips the primary key, so a
    /// client cannot choose a row's id by sending `attributes.id`.
    ///
    /// Relationships are optional in the document: whether one is *required*
    /// is a domain policy, not a question about the document's shape.
    pub fn for_create(schema: &ResourceSchema) -> Result<Self, crate::validation::BuildError> {
        let hidden: Vec<&str> = std::iter::once(schema.primary_key.as_str())
            .chain(
                schema
                    .relationships
                    .iter()
                    .map(|rel| rel.source_attribute.as_str()),
            )
            .collect();

        // `Mounted` supplies the `Schema` impl; the base path is unused when
        // building a validator, so any value does.
        let mounted = Mounted::new(schema, "");

        Self::builder(&mounted)
            .attributes(required_on_create(schema))
            .hide(hidden)
            .build()
    }
}

/// An `ash-domain` error, as a JSON:API error.
///
/// Most of what the domain returns is a legitimate outcome rather than a
/// fault, and reporting all of it as `500` would tell a client to retry a
/// request that will never succeed. The mapping:
///
/// | `ash_domain::Error` | Status |
/// |---|---|
/// | `NotFound` | `404` |
/// | `Forbidden` | `403` |
/// | `PolicyError` | `503` — an authorization *outage*, so retriable |
/// | `MissingTenant` | `400` |
/// | `Invalid` | `422`, with `source.pointer` at the named field |
/// | `Conflict` | `409` |
/// | `Contention` | `503` |
/// | `Closing` | `503` |
/// | `Unsupported` | `501` |
/// | `UnknownResource`, `UnknownAction` | `500` — a wiring bug |
/// | `DataLayer`, `Serialization` | `500`, message logged not sent |
///
/// The two `500`s stay opaque: a data-layer message can hold a connection
/// string or a fragment of a query, so it is logged against the correlation
/// id and never serialized.
pub fn error_from(err: &ash_domain::Error, ctx: &Context) -> JsonApiError {
    use ash_domain::Error;

    let code = match err {
        Error::NotFound(_) => crate::ErrorCode::NotFound,
        Error::Forbidden(_) => crate::ErrorCode::Forbidden,
        Error::PolicyError(_) => crate::ErrorCode::PolicyUnavailable,
        Error::MissingTenant(_) => crate::ErrorCode::MissingTenant,
        Error::Invalid { field, message } => {
            // `field` is an attribute path, which is what a pointer into the
            // document's `attributes` is built from.
            let pointer = match field {
                Some(field) => format!("/data/attributes/{}", field.replace('.', "/")),
                None => "/data".to_string(),
            };
            return JsonApiError::invalid(ctx, message.clone(), pointer);
        }
        Error::Conflict { .. } => crate::ErrorCode::Conflict,
        Error::Contention(_) => crate::ErrorCode::LockContention,
        Error::Closing(_) => crate::ErrorCode::Closing,
        Error::Unsupported(_) => crate::ErrorCode::Unsupported,
        // A resource or action the router offers but the domain does not
        // register is a wiring bug, not something a client can act on.
        Error::UnknownResource(_) | Error::UnknownAction { .. } => crate::ErrorCode::Internal,
        Error::DataLayer { .. } | Error::Serialization(_) => crate::ErrorCode::Internal,
    };

    let error = JsonApiError::in_request(code, ctx);

    // `detail` is a no-op for the sanitized codes, so this cannot leak a
    // data-layer message even though it is called unconditionally.
    error.detail(err.to_string())
}

/// Your state, as the `ash-domain` bridge sees it.
///
/// Implement it once, and [`domain_resource!`](crate::domain_resource!)
/// writes the [`Resource`](crate::crud::Resource) and per-operation impls for
/// every resource you mount — so the same [`api!`](crate::api!) that serves a
/// hand-rolled resource serves a domain-backed one.
///
/// ```ignore
/// impl DomainState for App {
///     type Backend = InMemoryDataLayer;
///
///     fn domain(&self) -> &Domain {
///         &self.domain
///     }
///
///     fn domain_context(&self, ctx: &Context) -> ash_domain::Context<Self::Backend> {
///         let mut dctx = ash_domain::Context::new(self.store.clone());
///         // Carry the request's principal and tenant into the domain, so
///         // policies see who is asking.
///         if let Some(tenant) = ctx.tenant() {
///             dctx.set_tenant(tenant);
///         }
///         dctx
///     }
/// }
/// ```
///
/// A fresh domain context per request is deliberate: it holds the actor and
/// tenant the policies evaluate against, and those belong to one request, not
/// to the process.
pub trait DomainState: Clone + Send + Sync + 'static {
    /// The data layer the domain reads and writes through.
    type Backend: ash_domain::context::Store + Send + Sync + 'static;

    /// The domain to run actions against.
    fn domain(&self) -> &ash_domain::Domain;

    /// A domain context for one request.
    ///
    /// Called once per action. Bind the request's principal and tenant here —
    /// whatever a policy needs to make its decision — from the JSON:API
    /// [`Context`], which carries them.
    fn domain_context(&self, ctx: &Context) -> ash_domain::Context<Self::Backend>;

    /// The mounted schema for one resource, by JSON:API type.
    ///
    /// Supplies the base path the domain does not know about. Written by
    /// [`domain_resource!`](crate::domain_resource!) via
    /// [`MountedSchemas`].
    fn mounted(&self, resource: &str) -> Option<Mounted<'_>>;
}

/// The schemas a [`DomainState`] serves, resolved once at startup.
///
/// [`Domain::schema`](ash_domain::Domain::schema) allocates, so resolving a
/// resource's schema per request would cost a full domain walk on every call.
/// This holds them alongside the base path each is mounted at.
///
/// ```ignore
/// let schemas = MountedSchemas::new(&domain, [("todo", "/todos")])?;
/// ```
pub struct MountedSchemas {
    schemas: std::collections::BTreeMap<String, (ResourceSchema, String)>,
}

impl MountedSchemas {
    /// Resolve each resource's schema and pair it with its base path.
    ///
    /// Fails naming the first resource the domain does not register, since a
    /// route mounted on a resource that does not exist should stop startup
    /// rather than 500 on its first request.
    pub fn new<'a>(
        domain: &ash_domain::Domain,
        mounts: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, String> {
        let schema = domain.schema();
        let mut schemas = std::collections::BTreeMap::new();

        for (name, base_path) in mounts {
            let resource = schema
                .resource(name)
                .ok_or_else(|| format!("the domain registers no resource named `{name}`"))?;

            schemas.insert(name.to_string(), (resource.clone(), base_path.to_string()));
        }

        Ok(Self { schemas })
    }

    /// The mounted schema for one resource.
    pub fn get(&self, resource: &str) -> Option<Mounted<'_>> {
        self.schemas
            .get(resource)
            .map(|(schema, base_path)| Mounted::new(schema, base_path))
    }
}

/// A request body on its way to the domain.
///
/// The domain reads a document into its own types, so the bridge carries the
/// validated JSON rather than a struct of its own — a second definition of
/// the resource is exactly what this integration exists to avoid.
#[derive(Debug, Clone)]
pub struct DomainInput(pub serde_json::Value);

/// Serve an `ash-domain` resource over JSON:API.
///
/// The ash-domain counterpart to [`resource!`](crate::resource!): where that
/// derives everything from a field list you write, this derives it from the
/// schema the domain already holds. Nothing about the resource is restated —
/// attributes, types, relationships and their cardinality all come from
/// `#[derive(Resource)]`.
///
/// ```ignore
/// ash_jsonapi::domain_resource! {
///     Todo as "todo" at "/todos",
///     state: App,
/// }
///
/// ash_jsonapi::api! {
///     operations: [ash_jsonapi::CRUD],
///     resource: Todo, name: "todo", path: "/todos", state: App,
/// }
/// ```
///
/// `App` implements [`DomainState`]; this writes
/// [`Resource`](crate::crud::Resource), [`Storage`](crate::crud::Storage) and
/// the five operation traits in terms of it, so [`api!`](crate::api!) serves a
/// domain resource exactly as it serves a hand-rolled one.
///
/// Every action runs through [`Bound`](ash_domain::domain::Bound) rather than
/// around it, so policies, extensions and domain validation all apply. A
/// `Forbidden` becomes `403`, an `Invalid` a `422` pointing at the field, and
/// a data-layer failure an opaque `500` — see [`error_from`].
/// A domain resource and its routes, from one declaration.
///
/// [`domain_resource!`](crate::domain_resource!) writes the traits and
/// [`api!`](crate::api!) the routes, and both need the same name, path and
/// state. Writing them twice is three chances for the two to disagree — and a
/// mismatched `name` is not a compile error, it is a resource that serializes
/// as one type and validates as another.
///
/// ```ignore
/// ash_jsonapi::domain_api! {
///     Todo as "todo" at "/todos",
///     state: App,
///     operations: [ash_jsonapi::CRUD],
/// }
/// ```
///
/// Exactly equivalent to the two macros written out, with each fact stated
/// once:
///
/// ```ignore
/// ash_jsonapi::domain_resource! {
///     Todo as "todo" at "/todos",
///     state: App,
/// }
///
/// ash_jsonapi::api! {
///     operations: [ash_jsonapi::CRUD],
///     resource: Todo, name: "todo", path: "/todos", state: App,
/// }
/// ```
///
/// Use the two directly when they need to diverge — a resource served at two
/// paths, or one whose traits you write by hand.
#[macro_export]
macro_rules! domain_api {
    (
        $resource:ident as $name:literal at $base:literal,
        state: $state:ident,
        operations: [$($op:tt)+]
        $(,)?
    ) => {
        $crate::domain_resource! {
            $resource as $name at $base,
            state: $state,
        }

        $crate::api! {
            operations: [$($op)+],
            resource: $resource,
            name: $name,
            path: $base,
            state: $state,
        }
    };
}

#[macro_export]
macro_rules! domain_resource {
    (
        $resource:ident as $name:literal at $base:literal,
        state: $state:ident
        $(,)?
    ) => {
        impl $crate::crud::Resource<$state> for $resource {
            type Create = $crate::domain::DomainInput;
            type Update = $crate::domain::DomainInput;

            const NAME: &'static str = $name;
            const PATH: &'static str = $base;

            fn to_resource(
                &self,
                state: &$state,
                ctx: &$crate::context::Context,
            ) -> ::std::result::Result<
                $crate::document::ResourceObject,
                $crate::error::JsonApiError,
            > {
                use $crate::domain::DomainState as _;
                use $crate::__ash_domain::value::IntoRecord as _;

                let mounted = state.mounted($name).ok_or_else(|| {
                    $crate::error::JsonApiError::internal(ctx)
                        .detail(::std::format!("`{}` is not mounted", $name))
                })?;

                // Serialized from the domain's own `Record`, so the primary
                // key and every foreign key land where JSON:API puts them
                // rather than beside the attributes.
                // Cloned because `IntoRecord` consumes the row, and
                // serialization borrows it — a handler may still need it.
                let record = ::std::clone::Clone::clone(self).into_record().map_err(|err| {
                    $crate::error::JsonApiError::internal(ctx).detail(err.to_string())
                })?;

                ::std::result::Result::Ok(mounted.record_from(&record).build())
            }
        }

        impl $crate::extract::FromRequest<$state> for $crate::domain::DomainInput {
            async fn from_request(
                request: ::axum::extract::Request,
                state: &$state,
                ctx: &$crate::context::Context,
            ) -> ::std::result::Result<Self, $crate::response::Errors> {
                let validator = $crate::crud::validator(state, $name, ctx)?;
                let document = $crate::extract::validated(request, ctx, validator).await?;

                ::std::result::Result::Ok($crate::domain::DomainInput(document))
            }
        }

        impl $crate::crud::Create<$resource> for $state {
            async fn create(
                &self,
                input: $crate::domain::DomainInput,
                ctx: &$crate::context::Context,
            ) -> ::std::result::Result<$resource, Self::Error> {
                use $crate::domain::DomainState as _;

                let mounted = self.mounted($name).ok_or_else(|| {
                    $crate::__ash_domain::Error::UnknownResource($name.to_string())
                })?;

                // The domain's own conversion reads the document, so the
                // resource is not described a second time here.
                let record: $crate::__ash_domain::value::Record = mounted
                    .record_for_create(&input.0, ctx)
                    .map_err(|err| $crate::__ash_domain::Error::invalid(err.to_string()))?;

                let mut dctx = self.domain_context(ctx);
                self.domain().bind(&mut dctx).create::<$resource>(record).await
            }
        }

        impl $crate::crud::Read<$resource> for $state {
            async fn get(
                &self,
                id: &str,
            ) -> ::std::result::Result<::std::option::Option<$resource>, Self::Error> {
                use $crate::domain::DomainState as _;

                // No request context reaches `get`, so the domain context is
                // built from an empty one: a read still runs every policy,
                // just without a principal bound.
                let ctx = $crate::context::Context::new("");
                let mut dctx = self.domain_context(&ctx);
                self.domain().bind(&mut dctx).get::<$resource>(id).await
            }
        }

        impl $crate::crud::Update<$resource> for $state {
            async fn update(
                &self,
                id: &str,
                input: $crate::domain::DomainInput,
                ctx: &$crate::context::Context,
            ) -> ::std::result::Result<$resource, Self::Error> {
                use $crate::domain::DomainState as _;

                let mounted = self.mounted($name).ok_or_else(|| {
                    $crate::__ash_domain::Error::UnknownResource($name.to_string())
                })?;

                let record: $crate::__ash_domain::value::Record = mounted
                    .record_for_update(&input.0, id, ctx)
                    .map_err(|err| $crate::__ash_domain::Error::invalid(err.to_string()))?;

                let mut dctx = self.domain_context(ctx);
                self.domain().bind(&mut dctx).update::<$resource>(id, record).await
            }
        }

        impl $crate::crud::Delete<$resource> for $state {
            async fn destroy(&self, id: &str) -> ::std::result::Result<(), Self::Error> {
                use $crate::domain::DomainState as _;

                let ctx = $crate::context::Context::new("");
                let mut dctx = self.domain_context(&ctx);
                self.domain().bind(&mut dctx).destroy::<$resource>(id).await
            }
        }

        impl $crate::crud::List<$resource> for $state {
            async fn list(
                &self,
                query: &$crate::query::Query,
                ctx: &$crate::context::Context,
            ) -> ::std::result::Result<$crate::crud::Listing<$resource>, Self::Error> {
                use $crate::domain::DomainState as _;

                // One row past the page, so `next` is a fact rather than a
                // guess: `Paginator` would otherwise infer "there is more"
                // from a full page and link the client into an empty one.
                let probed = $crate::domain::probe_query::<$resource>(query)?;

                let mut dctx = self.domain_context(ctx);
                let rows = self
                    .domain()
                    .bind(&mut dctx)
                    .read::<$resource>(probed.query)
                    .await?;

                ::std::result::Result::Ok($crate::domain::listing_from(rows, probed.size))
            }
        }

        impl $crate::crud::Storage<$resource> for $state {
            type Error = $crate::__ash_domain::Error;

            fn classify(
                &self,
                err: &Self::Error,
                ctx: &$crate::context::Context,
            ) -> ::std::option::Option<$crate::error::JsonApiError> {
                // Most domain outcomes are the client's business: a denial, a
                // conflict, an invalid field. `error_from` decides which, and
                // keeps the rest opaque.
                ::std::option::Option::Some($crate::domain::error_from(err, ctx))
            }
        }
    };
}

/// The domain's attribute schema, with a `required` list for a create body.
///
/// `ResourceSchema::to_json_schema` describes the attributes but marks none
/// required — the domain records a default, not a nullability flag, so there
/// is nothing for it to emit. Without a `required` list an empty `attributes`
/// object passes validation and fails deeper down, where the failure is no
/// longer attributable to a member.
///
/// The rule applied here: an attribute is required on create when it has no
/// default and is not the primary key. That is what the domain would itself
/// have to supply a value for.
#[cfg(feature = "validation")]
fn required_on_create(schema: &ResourceSchema) -> serde_json::Value {
    let mut json = schema.to_json_schema();

    let required: Vec<serde_json::Value> = schema
        .attributes
        .iter()
        .filter(|attr| !attr.primary_key && attr.default.is_none())
        .map(|attr| serde_json::Value::from(attr.name.as_str()))
        .collect();

    if let Some(object) = json.as_object_mut()
        && !required.is_empty()
    {
        object.insert("required".to_string(), serde_json::Value::Array(required));
    }

    json
}

/// A JSON:API collection request as a domain [`Query`](ash_domain::Query).
///
/// Offset paging maps directly onto the domain's own offset paging:
/// `page[offset]` becomes [`Query::offset`](ash_domain::Query::offset) and
/// `page[limit]` its [`limit`](ash_domain::Query::limit). Cursor paging does
/// not: the domain resumes from a [`Cursor`](ash_domain::query::Cursor) built
/// from the sort key's values, not from an opaque string, so a `page[cursor]`
/// this crate handed out cannot be turned back into one. Rather than
/// mistranslate it, a cursor request is refused.
///
/// Sort keys pass through by name; the domain rejects an attribute the
/// resource does not declare, which is where that check belongs.
///
/// # Why an offset implies a sort
///
/// The domain refuses an offset on an unsorted read, and it is right to:
/// skipping rows of an order nothing pinned down skips *arbitrary* rows, so
/// page 2 of an unsorted collection is not the page after page 1. But a bare
/// `GET /todos?page[offset]=25` carries no `?sort=`, and refusing it would
/// make the most ordinary paged request in JSON:API an error.
///
/// So a request that pages past the first row and names no sort is ordered by
/// the resource's primary key — the one attribute every resource has, and a
/// total order, which is what paging needs. A client that wants a different
/// order says so with `?sort=`, and that wins.
pub fn page_query<R: ash_domain::Resource>(
    query: &crate::query::Query,
) -> Result<ash_domain::Query, ash_domain::Error> {
    let mut domain_query = ash_domain::Query::new(R::NAME);

    for key in query.sort() {
        domain_query = if key.descending() {
            domain_query.sort_desc(key.field())
        } else {
            domain_query.sort_asc(key.field())
        };
    }

    match query.page() {
        crate::query::Page::Offset { offset, limit } => {
            let mut domain_query = domain_query.limit(*limit as u32);

            // Only a page past the first needs a position, and only a position
            // needs an order — a first page keeps the layer's natural order,
            // exactly as an unpaged read does.
            if *offset > 0 {
                // The domain counts offsets in `u32`. A larger one is past the
                // end of anything the domain can address, and casting would
                // wrap it into a small offset — silently serving some other
                // page. Refuse instead.
                let offset = u32::try_from(*offset).map_err(|_| {
                    ash_domain::Error::invalid(format!(
                        "`page[offset]` of {offset} is larger than this API can \
                         address (the maximum is {})",
                        u32::MAX
                    ))
                })?;

                if query.sort().is_empty() {
                    domain_query = domain_query.sort_asc(R::primary_key());
                }
                domain_query = domain_query.offset(offset);
            }

            Ok(domain_query)
        }
        crate::query::Page::Cursor { .. } => Err(ash_domain::Error::Unsupported(
            "this resource pages by `page[offset]`/`page[limit]`; \
             `page[cursor]` is not supported over ash-domain"
                .to_string(),
        )),
    }
}

/// A [`page_query`] that reads one row past the page.
///
/// Returned by [`probe_query`], and paired with [`listing_from`], which drops
/// the extra row and reports it as [`Listing::has_more`](crate::crud::Listing).
pub struct Probe {
    /// The query to run: the client's page, plus one row.
    pub query: ash_domain::Query,
    /// The page size the client actually asked for.
    pub size: usize,
}

/// A collection request as a domain query that reads **one row too many**.
///
/// `next` is otherwise a guess. A `Listing` that reports no total leaves the
/// [`Paginator`](crate::Paginator) inferring "there may be more" from a full
/// page, so the last full page links to an empty one and the client learns
/// the collection ended by fetching nothing. Reading one row past the page
/// answers it exactly, for the cost of one row rather than a `COUNT`.
///
/// This is the same trick ash-domain's
/// [`offset_page`](ash_domain::read::OffsetPage) plays, done here because the
/// erased read path returns rows rather than a page.
///
/// Pass the result to [`listing_from`], which puts the page back to the size
/// that was asked for.
pub fn probe_query<R: ash_domain::Resource>(
    query: &crate::query::Query,
) -> Result<Probe, ash_domain::Error> {
    let size = query.page().size();
    let mut probed = page_query::<R>(query)?;

    // Saturating, so a page size at the domain's ceiling widens to the
    // ceiling rather than wrapping to nothing.
    probed.limit = Some(probed.limit.unwrap_or(size as u32).saturating_add(1));

    Ok(Probe {
        query: probed,
        size,
    })
}

/// The rows of a [`probe_query`] as a [`Listing`](crate::crud::Listing).
///
/// Drops the probe row the client did not ask for, and reports its presence
/// as [`has_more`](crate::crud::Listing::has_more) — so the `next` link is
/// emitted exactly when there is a next page.
///
/// Gated with [`crud`](crate::crud) itself, which is where `Listing` lives:
/// a `domain`-only build has no HTTP surface to page.
#[cfg(all(feature = "http", feature = "validation"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "http", feature = "validation"))))]
pub fn listing_from<R>(mut rows: Vec<R>, size: usize) -> crate::crud::Listing<R> {
    let has_more = rows.len() > size;
    rows.truncate(size);
    crate::crud::Listing::new(rows).has_more(has_more)
}

impl Mounted<'_> {
    /// A create document as a domain [`Record`](ash_domain::value::Record).
    ///
    /// The document has already passed the validator, so what is left is the
    /// typed conversion the schema drives.
    pub fn record_for_create(
        &self,
        document: &serde_json::Value,
        ctx: &Context,
    ) -> Result<DomainRecord, JsonApiError> {
        self.read_document(document, ctx)
    }

    /// An update document as a domain
    /// [`Record`](ash_domain::value::Record), with the path's id.
    ///
    /// A `PATCH` addresses a row that exists, so the URL's id wins over
    /// anything the body carries.
    pub fn record_for_update(
        &self,
        document: &serde_json::Value,
        id: &str,
        ctx: &Context,
    ) -> Result<DomainRecord, JsonApiError> {
        let mut record = self.read_document(document, ctx)?;
        record
            .0
            .insert(self.schema.primary_key.clone(), Value::Str(id.to_string()));
        Ok(record)
    }
}
