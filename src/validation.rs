//! Validating request documents before they reach your code.
//!
//! JSON:API wraps a resource in a
//! `{"data":{"type","id","attributes","relationships"}}` envelope. This
//! module builds a JSON Schema for that envelope from your resource's
//! [`Schema`](crate::Schema) plus a JSON Schema for its attributes, and
//! checks request bodies against it.
//!
//! ```ignore
//! let validator = DocumentValidator::builder(&todo_schema)
//!     .attributes(serde_json::json!({
//!         "type": "object",
//!         "properties": { "message": { "type": "string" } },
//!         "required": ["message"],
//!     }))
//!     .build()?;
//!
//! let json = validator.parse(&body, &ctx)?;   // Vec<JsonApiError> on failure
//! ```
//!
//! The pay-off is the errors. Each failure is reported with its instance
//! location as a JSON Pointer, which is exactly what JSON:API's
//! `source.pointer` is — so one malformed body comes back as a complete error
//! document, pointing at every member that caused it, with no per-field
//! checks written by hand.
//!
//! # `format`
//!
//! JSON Schema 2020-12 treats `format` as an annotation, so `"format":
//! "email"` checks nothing by default. This crate asserts it instead: a
//! request schema that names a format is stating a rule, and a rule that
//! silently does nothing is worse than none. `date`, `date-time`, `time`,
//! `duration`, `email`, `idn-email`, `hostname`, `ipv4`, `ipv6`, `uuid`,
//! `json-pointer` and `relative-json-pointer` are checked;
//! [`ValidatorBuilder::format`] supplies a check for anything else, and
//! [`ValidatorBuilder::annotate_formats`] turns assertion off.
//!
//! With [`resource!`](crate::resource!), a field typed
//! [`Formatted<Email>`](crate::Formatted) declares and asserts the format in
//! one place.
//!
//! Using `ash-domain`? [`DocumentValidator::for_create`] derives the
//! attribute schema from a `ResourceSchema`, so there is nothing to write.

use ash_jsonschema::validator::CompileError;
use ash_jsonschema::validator::Mode;
use ash_jsonschema::{Schema, SchemaError, Validator};

use crate::context::Context;
use crate::error::JsonApiError;
use crate::serialize::{RelationshipDef, Schema as ResourceDef};

/// Why a validator could not be built from a resource schema.
///
/// A startup failure in every case: the schema is fixed once the domain is
/// registered, so this cannot fire at request time.
#[derive(Debug)]
pub enum BuildError {
    /// The derived schema is not valid JSON Schema 2020-12.
    Schema(SchemaError),
    /// `ash-domain`'s attribute schema did not deserialize.
    Attributes(serde_json::Error),
    /// The schema is valid but could not be compiled into a validator.
    Compile(CompileError),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schema(err) => write!(f, "derived schema is invalid: {err}"),
            Self::Attributes(err) => write!(f, "attribute schema did not parse: {err}"),
            Self::Compile(err) => write!(f, "schema did not compile: {err}"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<SchemaError> for BuildError {
    fn from(err: SchemaError) -> Self {
        Self::Schema(err)
    }
}

impl From<serde_json::Error> for BuildError {
    fn from(err: serde_json::Error) -> Self {
        Self::Attributes(err)
    }
}

impl From<CompileError> for BuildError {
    fn from(err: CompileError) -> Self {
        Self::Compile(err)
    }
}

/// Validates create documents for one resource.
pub struct DocumentValidator {
    validator: Validator,
}

/// A caller-supplied `format` check, as stored until the validator is built.
type FormatCheck = Box<dyn Fn(&str) -> bool + Send + Sync>;

/// Builds a [`DocumentValidator`] for one resource.
///
/// Created by [`DocumentValidator::builder`].
pub struct ValidatorBuilder<'a, S: ResourceDef> {
    schema: &'a S,
    attributes: serde_json::Value,
    hidden: Vec<String>,
    assert_formats: bool,
    formats: Vec<(String, FormatCheck)>,
}

impl<'a, S: ResourceDef> ValidatorBuilder<'a, S> {
    /// The JSON Schema an `attributes` object must satisfy.
    ///
    /// Anything JSON Schema 2020-12 accepts — usually an `object` with
    /// `properties` and `required`.
    #[must_use]
    pub fn attributes(mut self, schema: serde_json::Value) -> Self {
        self.attributes = schema;
        self
    }

    /// Drop these properties from the attribute schema.
    ///
    /// For fields your storage has but the wire format does not: the primary
    /// key (JSON:API carries it in the envelope) and any foreign key behind a
    /// relationship (carried as linkage).
    #[must_use]
    pub fn hide(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.hidden.extend(names.into_iter().map(Into::into));
        self
    }

    /// Stop asserting `format`, leaving it an annotation.
    ///
    /// JSON Schema 2020-12 makes `format` an annotation by default, and
    /// ash-jsonschema follows that. This crate turns assertion *on*, because
    /// a resource declaring `format: "email"` in a request schema means it
    /// wants the check — an unasserted `format` is a validation rule that
    /// silently does nothing.
    ///
    /// Turn it off when a schema is shared with a consumer that only
    /// annotates, so both ends agree on what is accepted.
    ///
    /// Asserted: `date`, `date-time`, `time`, `duration`, `email`,
    /// `idn-email`, `hostname`, `ipv4`, `ipv6`, `uuid`, `json-pointer` and
    /// `relative-json-pointer`. Any other name stays an annotation unless you
    /// give it a check with [`format`](Self::format).
    #[must_use]
    pub fn annotate_formats(mut self) -> Self {
        self.assert_formats = false;
        self
    }

    /// Assert one `format` with a check of your own.
    ///
    /// For a format the specification does not define — an internal id
    /// shape, a tenant-scoped key — or to replace a built-in check with a
    /// stricter one:
    ///
    /// ```ignore
    /// DocumentValidator::builder(&schema)
    ///     .attributes(attributes)
    ///     .format("employee-id", |value| {
    ///         value.strip_prefix("E-").is_some_and(|n| n.len() == 6)
    ///     })
    ///     .build()?
    /// ```
    ///
    /// The schema stays ordinary JSON Schema: another validator annotates
    /// `"format": "employee-id"` where this one asserts it. The check only
    /// ever sees strings, since `format` says nothing about other types.
    #[must_use]
    pub fn format<F>(mut self, name: impl Into<String>, check: F) -> Self
    where
        F: Fn(&str) -> bool + Send + Sync + 'static,
    {
        self.formats.push((name.into(), Box::new(check)));
        self
    }

    /// Compile the validator.
    ///
    /// Do this once at startup: a schema that will not compile should stop
    /// the service rather than surface as a 500 on the first request.
    pub fn build(mut self) -> Result<DocumentValidator, BuildError> {
        if let Some(properties) = self
            .attributes
            .get_mut("properties")
            .and_then(|p| p.as_object_mut())
        {
            properties.retain(|name, _| !self.hidden.iter().any(|hidden| hidden == name));
        }

        let mut resource = Schema::object()
            .required_property(
                "type",
                Schema::string().constant(self.schema.name().to_string()),
            )
            .required_property("attributes", Schema::from_value(self.attributes)?);

        let relationships = self.schema.relationships();
        if !relationships.is_empty() {
            resource = resource.property("relationships", relationships_schema(relationships)?);
        }

        let document = Schema::object()
            .required_property("data", resource.build()?)
            .build()?;

        // Formats are asserted rather than annotated: a resource that
        // declares `format: "email"` on a request attribute is stating a
        // rule, and an unasserted format is a rule that does nothing.
        let mut validator = Validator::builder(&document);
        if self.assert_formats {
            validator = validator.assert_formats();
        }
        for (name, check) in self.formats {
            validator = validator.format(name, move |value| check(value));
        }

        Ok(DocumentValidator {
            validator: validator.build()?,
        })
    }
}

impl DocumentValidator {
    /// Start building a validator for `schema`.
    pub fn builder<S: ResourceDef>(schema: &S) -> ValidatorBuilder<'_, S> {
        ValidatorBuilder {
            schema,
            attributes: serde_json::json!({ "type": "object" }),
            hidden: Vec::new(),
            assert_formats: true,
            formats: Vec::new(),
        }
    }

    /// Parse a raw body and check it in one step.
    ///
    /// The usual entry point: a body that is not JSON at all is a 422 like
    /// any other malformed document, so a handler has one failure path
    /// rather than two.
    pub fn parse(&self, body: &str, ctx: &Context) -> Result<serde_json::Value, Vec<JsonApiError>> {
        let json: serde_json::Value = serde_json::from_str(body).map_err(|err| {
            vec![JsonApiError::invalid(
                ctx,
                format!("malformed JSON: {err}"),
                "",
            )]
        })?;

        self.check(&json, ctx)?;
        Ok(json)
    }

    /// Check one request body.
    ///
    /// Every failure becomes a JSON:API error carrying the pointer at the
    /// member that caused it and the request's correlation id.
    pub fn check(&self, body: &serde_json::Value, ctx: &Context) -> Result<(), Vec<JsonApiError>> {
        let outcome = self.validator.validate(body, Mode::All);
        if outcome.errors.is_empty() {
            return Ok(());
        }

        Err(outcome
            .errors
            .iter()
            .map(|error| {
                // The validator's instance location is already a JSON Pointer
                // into the document, which is what source.pointer wants.
                JsonApiError::invalid(ctx, error.message(), error.instance_location.clone())
            })
            .collect())
    }
}

/// `relationships`: one optional linkage object per declared relationship.
fn relationships_schema<R: RelationshipDef>(defs: &[R]) -> Result<Schema, SchemaError> {
    let mut relationships = Schema::object();

    for rel in defs {
        let identifier = Schema::object()
            .required_property("type", Schema::string().constant(rel.target().to_string()))
            .required_property("id", Schema::string().min_length(1))
            .build()?;

        // A to-many relationship's linkage is an array of identifiers; a
        // to-one is a single object. The schema says which, so a client
        // sending the wrong shape is told exactly that.
        let data = if rel.is_to_many() {
            Schema::array().items(identifier).build()?
        } else {
            identifier
        };

        relationships = relationships.property(
            rel.name().to_string(),
            Schema::object().required_property("data", data).build()?,
        );
    }

    relationships.build()
}
