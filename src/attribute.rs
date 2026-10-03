//! Attribute types: one Rust type, its JSON Schema, and how it is read back.
//!
//! [`resource!`](crate::resource!) needs three things for every field it is
//! given: the JSON Schema that describes it, how to read it out of a
//! validated document, and how to write it back into a response. Rather than
//! matching on type names in the macro — which would only ever know the types
//! the macro was written for — each is a method on this trait.
//!
//! Implementations ship for [`String`], the integer and float types, [`bool`],
//! and [`Option<T>`] over any of them. Implement it yourself for a newtype, an
//! enum, or anything else you want a resource to carry:
//!
//! ```ignore
//! struct Priority(u8);
//!
//! impl Attribute for Priority {
//!     fn schema() -> serde_json::Value {
//!         serde_json::json!({ "type": "integer", "minimum": 1, "maximum": 5 })
//!     }
//!
//!     fn read(value: &serde_json::Value) -> Option<Self> {
//!         value.as_u64().map(|n| Priority(n as u8))
//!     }
//!
//!     fn write(&self) -> serde_json::Value {
//!         serde_json::Value::from(self.0)
//!     }
//! }
//! ```
//!
//! Validation still happens against the schema before `read` runs, so `read`
//! is reached only for a value the schema already accepted.

/// A type a resource attribute can hold.
pub trait Attribute: Sized {
    /// The JSON Schema for this type, as it appears under `attributes`.
    fn schema() -> serde_json::Value;

    /// Read the value out of a validated document.
    ///
    /// `None` means the value was not of this type. The schema has already
    /// rejected anything ill-typed, so this fires only for a type whose
    /// schema is looser than its `read` — report it rather than defaulting.
    fn read(value: &serde_json::Value) -> Option<Self>;

    /// Write the value into a response's `attributes`.
    fn write(&self) -> serde_json::Value;

    /// Whether the member may be absent.
    ///
    /// Only [`Option`] overrides this. It drives the `required` list of the
    /// generated schema, so an optional field is one that may be omitted.
    fn required() -> bool {
        true
    }
}

/// A string attribute in a declared JSON Schema `format`.
///
/// The wrapper that puts `format` to work: a resource field typed
/// `Formatted<Email>` generates `{"type": "string", "format": "email"}`, and
/// the validator asserts it — so a malformed address is a `422` with a
/// pointer at the member, checked before any of your code runs.
///
/// ```text
/// ash_jsonapi::resource! {
///     User as "user" at "/users",
///     …
///     attributes: {
///         email: Formatted<Email>,
///         joined: Formatted<Date>,
///     },
/// }
/// ```
///
/// The inner value is a [`String`]; the type parameter only carries which
/// format to declare. For a format the specification does not define, give
/// the validator a check with
/// [`ValidatorBuilder::format`](crate::validation::ValidatorBuilder::format)
/// and write your own [`FormatName`].
// The derives are written out rather than `#[derive]`d: a derive would bound
// each impl on `F`, and `F` is a marker that never exists at runtime. A user's
// format type should not have to be `Clone` for their resource to be.
pub struct Formatted<F: FormatName> {
    value: String,
    format: std::marker::PhantomData<F>,
}

impl<F: FormatName> Clone for Formatted<F> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            format: std::marker::PhantomData,
        }
    }
}

impl<F: FormatName> std::fmt::Debug for Formatted<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Shown as the format it declares, so a log line says what failed.
        write!(f, "Formatted<{}>({:?})", F::NAME, self.value)
    }
}

impl<F: FormatName> Default for Formatted<F> {
    fn default() -> Self {
        Self {
            value: String::new(),
            format: std::marker::PhantomData,
        }
    }
}

impl<F: FormatName> PartialEq for Formatted<F> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<F: FormatName> Eq for Formatted<F> {}

impl<F: FormatName> PartialOrd for Formatted<F> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<F: FormatName> Ord for Formatted<F> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<F: FormatName> std::hash::Hash for Formatted<F> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<F: FormatName> Formatted<F> {
    /// The string itself.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Take the string out.
    pub fn into_inner(self) -> String {
        self.value
    }
}

impl<F: FormatName> std::fmt::Display for Formatted<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value)
    }
}

impl<F: FormatName> Attribute for Formatted<F> {
    fn schema() -> serde_json::Value {
        serde_json::json!({ "type": "string", "format": F::NAME })
    }

    fn read(value: &serde_json::Value) -> Option<Self> {
        // The format itself is the validator's to check, and it already has
        // by the time this runs.
        value.as_str().map(|value| Self {
            value: value.to_string(),
            format: std::marker::PhantomData,
        })
    }

    fn write(&self) -> serde_json::Value {
        serde_json::Value::from(self.value.clone())
    }
}

/// The name of a JSON Schema `format`, as it appears in a schema.
///
/// Implement it on a marker type to declare a format this crate does not
/// ship — pairing it with
/// [`ValidatorBuilder::format`](crate::validation::ValidatorBuilder::format),
/// which supplies the check:
///
/// ```ignore
/// struct EmployeeId;
/// impl FormatName for EmployeeId { const NAME: &'static str = "employee-id"; }
/// ```
pub trait FormatName {
    /// The `format` keyword's value, e.g. `"email"`.
    const NAME: &'static str;
}

/// The formats ash-jsonschema asserts, as marker types for [`Formatted`].
///
/// Only these are checked without supplying your own function; any other name
/// is an annotation until you give it one.
macro_rules! formats {
    ($($(#[$doc:meta])* $name:ident => $format:literal),+ $(,)?) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
            pub struct $name;

            impl FormatName for $name {
                const NAME: &'static str = $format;
            }
        )+
    };
}

formats! {
    /// RFC 3339 timestamp, e.g. `2024-02-29T12:00:00Z`.
    DateTime => "date-time",
    /// RFC 3339 full-date, e.g. `2024-02-29`.
    Date => "date",
    /// RFC 3339 full-time.
    Time => "time",
    /// ISO 8601 duration, e.g. `P3DT4H`.
    Duration => "duration",
    /// RFC 5321 mailbox.
    Email => "email",
    /// RFC 6531 internationalized mailbox.
    IdnEmail => "idn-email",
    /// RFC 1123 host name.
    Hostname => "hostname",
    /// Dotted-quad address.
    Ipv4 => "ipv4",
    /// RFC 4291 address.
    Ipv6 => "ipv6",
    /// RFC 4122 UUID.
    Uuid => "uuid",
    /// RFC 6901 JSON Pointer.
    JsonPointer => "json-pointer",
    /// Relative JSON Pointer.
    RelativeJsonPointer => "relative-json-pointer",
}

/// `String` → `{"type": "string"}`.
impl Attribute for String {
    fn schema() -> serde_json::Value {
        serde_json::json!({ "type": "string" })
    }

    fn read(value: &serde_json::Value) -> Option<Self> {
        value.as_str().map(ToString::to_string)
    }

    fn write(&self) -> serde_json::Value {
        serde_json::Value::from(self.clone())
    }
}

/// `bool` → `{"type": "boolean"}`.
impl Attribute for bool {
    fn schema() -> serde_json::Value {
        serde_json::json!({ "type": "boolean" })
    }

    fn read(value: &serde_json::Value) -> Option<Self> {
        value.as_bool()
    }

    fn write(&self) -> serde_json::Value {
        serde_json::Value::from(*self)
    }
}

/// The integer types, each with the bounds its width actually has.
///
/// A `u8` field rejects `300` at validation rather than wrapping or failing
/// later, because the schema carries the real range.
macro_rules! integer_attribute {
    ($($ty:ty => $as:ident),+ $(,)?) => {
        $(
            impl Attribute for $ty {
                fn schema() -> serde_json::Value {
                    serde_json::json!({
                        "type": "integer",
                        "minimum": <$ty>::MIN,
                        "maximum": <$ty>::MAX,
                    })
                }

                fn read(value: &serde_json::Value) -> Option<Self> {
                    value.$as().and_then(|n| <$ty>::try_from(n).ok())
                }

                fn write(&self) -> serde_json::Value {
                    serde_json::Value::from(*self)
                }
            }
        )+
    };
}

integer_attribute! {
    i8 => as_i64, i16 => as_i64, i32 => as_i64, i64 => as_i64,
    u8 => as_u64, u16 => as_u64, u32 => as_u64, u64 => as_u64,
}

/// The float types → `{"type": "number"}`.
macro_rules! float_attribute {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl Attribute for $ty {
                fn schema() -> serde_json::Value {
                    serde_json::json!({ "type": "number" })
                }

                fn read(value: &serde_json::Value) -> Option<Self> {
                    value.as_f64().map(|n| n as $ty)
                }

                fn write(&self) -> serde_json::Value {
                    serde_json::Value::from(*self)
                }
            }
        )+
    };
}

float_attribute!(f32, f64);

/// `Option<T>` — the member may be absent, and `null` reads as `None`.
impl<T: Attribute> Attribute for Option<T> {
    fn schema() -> serde_json::Value {
        T::schema()
    }

    fn read(value: &serde_json::Value) -> Option<Self> {
        // Absent and explicit null are the same thing here: no value.
        if value.is_null() {
            return Some(None);
        }
        T::read(value).map(Some)
    }

    fn write(&self) -> serde_json::Value {
        match self {
            Some(value) => value.write(),
            None => serde_json::Value::Null,
        }
    }

    fn required() -> bool {
        false
    }
}
