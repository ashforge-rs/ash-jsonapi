//! Declaring a resource once.
//!
//! Without this, the same field list is written four times: the
//! [`Schema`](crate::Schema), the [`Resource`](crate::crud::Resource) impl
//! that serializes it, the [`FromRequest`](crate::extract::FromRequest) impl
//! that reads a body back, and the JSON Schema those attributes must satisfy.
//! Four copies that have to agree, with nothing checking that they do —
//! rename a field and the compiler stays happy while requests fail at
//! runtime.
//!
//! [`resource!`](crate::resource!) takes the list once and writes all four.

use crate::context::Context;
use crate::error::JsonApiError;

/// Read one attribute out of a validated document.
///
/// Used by [`resource!`](crate::resource!). The schema has already checked
/// the member's type, so a failure here means the two disagree — a
/// hand-written [`Attribute`](crate::attribute::Attribute) whose `schema` is
/// looser than its `read` — and it is reported as a `422` pointing at the
/// member rather than quietly defaulted.
pub fn read_attribute<T: crate::attribute::Attribute>(
    document: &serde_json::Value,
    name: &str,
    ctx: &Context,
) -> Result<T, JsonApiError> {
    let value = document
        .pointer(&format!("/data/attributes/{name}"))
        .unwrap_or(&serde_json::Value::Null);

    T::read(value).ok_or_else(|| {
        JsonApiError::invalid(
            ctx,
            format!("`{name}` is not of the expected type"),
            format!("/data/attributes/{name}"),
        )
    })
}

/// Read one relationship's id out of a validated document.
///
/// Absent linkage is `None`, so a create that omits an optional relationship
/// is not an error here — whether it is required is the schema's business.
pub fn read_relationship(document: &serde_json::Value, name: &str) -> Option<String> {
    document
        .pointer(&format!("/data/relationships/{name}/data/id"))
        .and_then(|id| id.as_str())
        .map(ToString::to_string)
}

/// Read a to-many relationship's ids out of a validated document.
pub fn read_relationships(document: &serde_json::Value, name: &str) -> Vec<String> {
    document
        .pointer(&format!("/data/relationships/{name}/data"))
        .and_then(|data| data.as_array())
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.get("id").and_then(|id| id.as_str()))
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Declare a resource once: its schema, its serialization, and its parsing.
///
/// ```ignore
/// ash_jsonapi::resource! {
///     Todo as "todo" at "/todos",
///     state: App,
///     schema: TodoSchema,
///     input: NewTodo,
///     id: id,
///     attributes: { message: String, done: bool },
/// }
/// ```
///
/// That one declaration expands to everything you would otherwise write by
/// hand:
///
/// - `struct TodoSchema` and its [`Schema`](crate::Schema) impl — the name,
///   the base path, and the relationships.
/// - `struct NewTodo`, the parsed body, with a public field per attribute,
///   plus its [`FromRequest`](crate::extract::FromRequest) impl: validate
///   against the resource's registered validator, then read each field.
/// - `impl Resource<App> for Todo` — `NAME`, `PATH`, and a `to_resource`
///   writing every attribute and relationship.
/// - `Todo::attributes_schema()`, the JSON Schema for the attributes, built
///   from each field's [`Attribute`](crate::attribute::Attribute) impl.
///
/// The field list appears once, so those four cannot drift apart. Field types
/// need [`Attribute`](crate::attribute::Attribute) — implemented already for
/// the standard scalars, `Option<T>` over them, and anything you implement it
/// on yourself.
///
/// The `schema:` and `input:` names are yours to pick. A `macro_rules!` macro
/// cannot build one identifier from another, so defaulting them to
/// `TodoSchema`/`NewTodo` would mean either a `paste`-style dependency in
/// every user's tree or a companion proc-macro crate — more than two lines of
/// ceremony is worth. They are also the names *you* write when you reference
/// the generated types, so naming them here keeps that explicit.
/// `TodoSchema` and `NewTodo` are the conventional shapes.
///
/// # Relationships
///
/// ```ignore
/// ash_jsonapi::resource! {
///     Todo as "todo" at "/todos",
///     state: App,
///     schema: TodoSchema,
///     input: NewTodo,
///     id: id,
///     attributes: { message: String },
///     relationships: { owner -> one "user", tags -> many "tag" },
/// }
/// ```
///
/// Target type and arity are declared once and reach the linkage, both link
/// URLs, and the request schema. `-> one` reads a field holding one id;
/// `-> many` a field holding a collection of them. On the input struct a
/// to-one becomes `Option<String>` — absent linkage is not an error — and a
/// to-many becomes `Vec<String>`.
///
/// # What it does not write
///
/// The storage impls, and the validator's registration. Where rows live is
/// yours: this writes the wire format, not your data layer. Pair it with
/// [`api!`](crate::api!) for the routes.
///
/// Need something it does not express — a computed attribute, an attribute
/// whose name differs from its field, a body that is not JSON:API at all?
/// Write the impls by hand. They are three ordinary traits, and this is
/// shorthand for the common shape rather than the only way in.
#[macro_export]
macro_rules! resource {
    (
        $resource:ident as $name:literal at $base:literal,
        state: $state:ident,
        schema: $schema:ident,
        input: $input:ident,
        id: $id:ident,
        attributes: { $($field:ident : $ty:ty),* $(,)? }
        $(, relationships: { $($rel:ident -> $arity:ident $target:literal),* $(,)? })?
        $(,)?
    ) => {
        #[doc = ::std::concat!("The JSON:API schema for [`", ::std::stringify!($resource), "`].")]
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $schema;

        impl $crate::serialize::Schema for $schema {
            type Relationship = $crate::serialize::Rel;

            fn name(&self) -> &str {
                $name
            }

            fn base_path(&self) -> &str {
                $base
            }

            fn relationships(&self) -> &[$crate::serialize::Rel] {
                // A `const` rather than an inline array: the slice outlives
                // the call, and `Rel`'s constructors are `const` for exactly
                // this.
                const RELATIONSHIPS: &[$crate::serialize::Rel] =
                    &[$($($crate::__rel_def!($rel, $arity, $target)),*)?];
                RELATIONSHIPS
            }
        }

        #[doc = ::std::concat!("A validated request body for [`", ::std::stringify!($resource), "`].")]
        #[derive(Debug, Clone)]
        pub struct $input {
            $(
                #[doc = ::std::concat!("The `", ::std::stringify!($field), "` attribute.")]
                pub $field: $ty,
            )*
            $($(
                #[doc = ::std::concat!("Linkage for the `", ::std::stringify!($rel), "` relationship.")]
                pub $rel: $crate::__rel_input_ty!($arity),
            )*)?
        }

        impl $crate::extract::FromRequest<$state> for $input {
            async fn from_request(
                request: ::axum::extract::Request,
                state: &$state,
                ctx: &$crate::context::Context,
            ) -> ::std::result::Result<Self, $crate::response::Errors> {
                let validator = $crate::crud::validator(state, $name, ctx)?;
                let document = $crate::extract::validated(request, ctx, validator).await?;

                // Every attribute is read before any failure is returned, so
                // a body with two bad members reports both — the same reason
                // validation runs in `Mode::All`.
                #[allow(unused_mut)]
                let mut failures: ::std::vec::Vec<$crate::error::JsonApiError> =
                    ::std::vec::Vec::new();

                $(
                    let $field = match $crate::declare::read_attribute::<$ty>(
                        &document,
                        ::std::stringify!($field),
                        ctx,
                    ) {
                        ::std::result::Result::Ok(value) => ::std::option::Option::Some(value),
                        ::std::result::Result::Err(err) => {
                            failures.push(err);
                            ::std::option::Option::None
                        }
                    };
                )*

                if !failures.is_empty() {
                    return ::std::result::Result::Err($crate::response::Errors(failures));
                }

                ::std::result::Result::Ok(Self {
                    $(
                        // Unreachable while `failures` is empty: the read
                        // either produced a value or pushed a failure.
                        $field: $field.expect("read succeeded"),
                    )*
                    $($(
                        $rel: $crate::__rel_read!(document, $rel, $arity),
                    )*)?
                })
            }
        }

        impl $resource {
            /// The JSON Schema this resource's `attributes` must satisfy.
            ///
            /// Built from each field's `Attribute` impl. Hand it to
            /// `DocumentValidator::builder(…).attributes(…)`.
            pub fn attributes_schema() -> $crate::__serde_json::Value {
                #[allow(unused_mut)]
                let mut properties = $crate::__serde_json::Map::new();
                #[allow(unused_mut)]
                let mut required: ::std::vec::Vec<$crate::__serde_json::Value> = ::std::vec::Vec::new();

                $(
                    properties.insert(
                        ::std::string::String::from(::std::stringify!($field)),
                        <$ty as $crate::attribute::Attribute>::schema(),
                    );
                    if <$ty as $crate::attribute::Attribute>::required() {
                        required.push($crate::__serde_json::Value::from(::std::stringify!($field)));
                    }
                )*

                $crate::__serde_json::json!({
                    "type": "object",
                    "properties": $crate::__serde_json::Value::Object(properties),
                    "required": $crate::__serde_json::Value::Array(required),
                })
            }

            /// A validator for this resource's request bodies.
            ///
            /// The whole of what registering a resource takes: build it at
            /// startup, hand it back from [`Validators::validator`](crate::crud::Validators::validator).
            pub fn validator() -> ::std::result::Result<
                $crate::validation::DocumentValidator,
                $crate::validation::BuildError,
            > {
                $crate::validation::DocumentValidator::builder(&$schema)
                    .attributes(Self::attributes_schema())
                    .build()
            }
        }

        impl $crate::crud::Resource<$state> for $resource {
            type Create = $input;
            type Update = $input;

            const NAME: &'static str = $name;
            const PATH: &'static str = $base;

            fn to_resource(
                &self,
                _state: &$state,
                _ctx: &$crate::context::Context,
            ) -> ::std::result::Result<$crate::document::ResourceObject, $crate::error::JsonApiError>
            {
                #[allow(unused_mut)]
                // `record_owned`, not `record`: the id is rendered here and
                // handed over, rather than rendered, lent, and rendered again.
                let mut record = $crate::serialize::Serializer::new(&$schema)
                    .record_owned(::std::string::ToString::to_string(&self.$id));

                $(
                    record = record.attr(
                        ::std::stringify!($field),
                        <$ty as $crate::attribute::Attribute>::write(&self.$field),
                    );
                )*

                $($(
                    record = $crate::__rel_write!(record, self, $rel, $arity);
                )*)?

                ::std::result::Result::Ok(record.build())
            }
        }
    };
}

/// One relationship's `Rel`, by arity.
#[doc(hidden)]
#[macro_export]
macro_rules! __rel_def {
    ($rel:ident, one, $target:literal) => {
        $crate::serialize::Rel::to_one(::std::stringify!($rel), $target)
    };
    ($rel:ident, many, $target:literal) => {
        $crate::serialize::Rel::to_many(::std::stringify!($rel), $target)
    };
    ($rel:ident, $other:ident, $target:literal) => {
        ::std::compile_error!(::std::concat!(
            "unknown relationship arity `",
            ::std::stringify!($other),
            "`; expected `one` or `many`"
        ))
    };
}

/// The input struct's field type for one relationship.
#[doc(hidden)]
#[macro_export]
macro_rules! __rel_input_ty {
    (one) => { ::std::option::Option<::std::string::String> };
    (many) => { ::std::vec::Vec<::std::string::String> };
    ($other:ident) => {
        ::std::compile_error!(::std::concat!(
            "unknown relationship arity `",
            ::std::stringify!($other),
            "`; expected `one` or `many`"
        ))
    };
}

/// Reading one relationship out of a validated document, by arity.
#[doc(hidden)]
#[macro_export]
macro_rules! __rel_read {
    ($document:ident, $rel:ident, one) => {
        $crate::declare::read_relationship(&$document, ::std::stringify!($rel))
    };
    ($document:ident, $rel:ident, many) => {
        $crate::declare::read_relationships(&$document, ::std::stringify!($rel))
    };
}

/// Writing one relationship's linkage, by arity.
#[doc(hidden)]
#[macro_export]
macro_rules! __rel_write {
    ($record:ident, $self:ident, $rel:ident, one) => {
        $record.rel(
            ::std::stringify!($rel),
            ::std::string::ToString::to_string(&$self.$rel),
        )
    };
    ($record:ident, $self:ident, $rel:ident, many) => {
        $record.rel_many(
            ::std::stringify!($rel),
            $self.$rel.iter().map(::std::string::ToString::to_string),
        )
    };
}

/// Implement [`Validators`](crate::crud::Validators) from a list of resources.
///
/// The validators have to live somewhere on your state, and looking them up
/// by name is the same match every time:
///
/// ```ignore
/// ash_jsonapi::registry! {
///     App { todos: Todo, users: User }
/// }
/// ```
///
/// `todos` and `users` are fields on your state holding a
/// [`DocumentValidator`](crate::validation::DocumentValidator) — or anything
/// that derefs to one, which is usually what you want, since your state has
/// to be [`Clone`] and a validator is not. The names they answer to are the
/// resources' own `NAME`. Build the fields with the generated
/// `Todo::validator()` at startup:
///
/// ```ignore
/// let state = App {
///     todos: Arc::new(Todo::validator()?),
///     users: Arc::new(User::validator()?),
///     /* … the rest of your state … */
/// };
/// ```
///
/// # With hooks
///
/// A state that carries hooks as well as validators names the field after
/// the resource list:
///
/// ```ignore
/// ash_jsonapi::registry! {
///     App { todos: Todo, users: User }
///     hooks: hooks
/// }
/// ```
///
/// Without this arm the macro writes the whole
/// [`Validators`](crate::crud::Validators) impl, leaving nowhere to add
/// [`hooks`](crate::crud::Validators::hooks) — so a service wanting both had
/// to hand-write the lookup.
///
/// A resource whose bodies you never parse — a read-only one — needs no
/// entry. A service where *nothing* parses a body needs no `registry!` at
/// all: [`Validators::validator`](crate::crud::Validators::validator) defaults to
/// `None`, so `impl Validators for App {}` is the whole wiring.
///
/// Omitting an entry for a resource you *do* route `CREATE` or `UPDATE` for
/// is a wiring bug, and reported as one — the request fails naming the
/// resource, rather than skipping validation.
#[macro_export]
macro_rules! registry {
    // With a hooks field: the state carries both, which is the shape a
    // service with validators *and* an audit trail needs. Spelled as a
    // separate arm rather than an optional trailing field so the common case
    // stays a one-liner.
    ($state:ident { $($field:ident : $resource:ty),* $(,)? } hooks: $hooks:ident $(,)?) => {
        $crate::__registry_impl!($state { $($field : $resource),* } hooks: $hooks);
    };

    ($state:ident { $($field:ident : $resource:ty),* $(,)? }) => {
        $crate::__registry_impl!($state { $($field : $resource),* });
    };
}

/// The `Validators` impl both [`registry!`](crate::registry!) arms write.
#[doc(hidden)]
#[macro_export]
macro_rules! __registry_impl {
    ($state:ident { $($field:ident : $resource:ty),* } $(hooks: $hooks:ident)?) => {
        impl $crate::crud::Validators for $state {
            $(
                fn hooks(&self) -> ::std::option::Option<&$crate::hook::Hooks> {
                    ::std::option::Option::Some(&self.$hooks)
                }
            )?

            fn validator(
                &self,
                resource: &str,
            ) -> ::std::option::Option<&$crate::validation::DocumentValidator> {
                match resource {
                    $(
                        <$resource as $crate::crud::Resource<$state>>::NAME => {
                            // `&*` so the field may be a validator, an
                            // `Arc<DocumentValidator>`, or anything else that
                            // derefs to one — state must be `Clone`, and a
                            // bare validator is not.
                            ::std::option::Option::Some(&*self.$field)
                        }
                    )*
                    _ => ::std::option::Option::None,
                }
            }
        }
    };
}
