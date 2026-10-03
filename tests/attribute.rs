//! `Attribute`: the schema a field declares, and reading it back.
//!
//! The schema is what the validator checks a request against, so a wrong
//! bound here is a request accepted that should not have been.
use ash_jsonapi::attribute::{Date, Email, Uuid};
use ash_jsonapi::{Attribute, FormatName, Formatted};

#[test]
fn a_string_declares_a_string() {
    assert_eq!(String::schema(), serde_json::json!({ "type": "string" }));
    assert_eq!(String::read(&serde_json::json!("hi")).unwrap(), "hi");
    assert_eq!("hi".to_string().write(), serde_json::json!("hi"));
}

#[test]
fn a_bool_declares_a_boolean() {
    assert_eq!(bool::schema(), serde_json::json!({ "type": "boolean" }));
    assert_eq!(bool::read(&serde_json::json!(true)), Some(true));
    assert_eq!(false.write(), serde_json::json!(false));
}

/// The bounds are the type's real range, so `300` in a `u8` is refused at
/// validation rather than wrapping later.
#[test]
fn an_integer_declares_the_range_its_width_actually_has() {
    let schema = u8::schema();
    assert_eq!(schema["type"], "integer");
    assert_eq!(schema["minimum"], 0);
    assert_eq!(schema["maximum"], 255);

    let schema = i8::schema();
    assert_eq!(schema["minimum"], -128);
    assert_eq!(schema["maximum"], 127);

    let schema = i64::schema();
    assert_eq!(schema["minimum"], i64::MIN);
    assert_eq!(schema["maximum"], i64::MAX);
}

#[test]
fn an_out_of_range_integer_does_not_read() {
    // The schema would have refused it; `read` must not wrap or truncate.
    assert_eq!(u8::read(&serde_json::json!(300)), None);
    assert_eq!(i8::read(&serde_json::json!(-200)), None);
    assert_eq!(u8::read(&serde_json::json!(255)), Some(255));
}

#[test]
fn a_negative_value_does_not_read_as_unsigned() {
    assert_eq!(u32::read(&serde_json::json!(-1)), None);
}

#[test]
fn a_wrongly_typed_value_reads_as_none() {
    assert_eq!(String::read(&serde_json::json!(1)), None);
    assert_eq!(bool::read(&serde_json::json!("true")), None);
    assert_eq!(u8::read(&serde_json::json!("1")), None);
}

#[test]
fn a_float_declares_a_number() {
    assert_eq!(f64::schema(), serde_json::json!({ "type": "number" }));
    assert_eq!(f64::read(&serde_json::json!(1.5)), Some(1.5));
    assert_eq!(f32::read(&serde_json::json!(1.5)), Some(1.5f32));
}

#[test]
fn every_declared_type_is_required_except_option() {
    assert!(String::required());
    assert!(bool::required());
    assert!(u8::required());
    assert!(f64::required());
    assert!(
        !Option::<String>::required(),
        "an optional field may be absent"
    );
}

#[test]
fn an_option_borrows_the_inner_schema() {
    // The optionality lives in `required`, not in the type keyword.
    assert_eq!(Option::<String>::schema(), String::schema());
    assert_eq!(Option::<u8>::schema(), u8::schema());
}

#[test]
fn null_reads_as_none_and_a_value_as_some() {
    assert_eq!(Option::<String>::read(&serde_json::json!(null)), Some(None));
    assert_eq!(
        Option::<String>::read(&serde_json::json!("hi")),
        Some(Some("hi".to_string()))
    );
    // Still ill-typed: not null, and not a string.
    assert_eq!(Option::<String>::read(&serde_json::json!(1)), None);
}

#[test]
fn an_option_writes_null_when_empty() {
    assert_eq!(None::<String>.write(), serde_json::Value::Null);
    assert_eq!(Some("hi".to_string()).write(), serde_json::json!("hi"));
}

#[test]
fn a_nested_option_still_reads() {
    assert_eq!(
        Option::<Option<String>>::read(&serde_json::json!(null)),
        Some(None)
    );
}

// --- Formatted ---------------------------------------------------------

#[test]
fn a_formatted_string_declares_its_format() {
    assert_eq!(
        Formatted::<Email>::schema(),
        serde_json::json!({ "type": "string", "format": "email" })
    );
    assert_eq!(Formatted::<Date>::schema()["format"], "date");
    assert_eq!(Formatted::<Uuid>::schema()["format"], "uuid");
}

#[test]
fn a_formatted_value_round_trips_as_a_plain_string() {
    let value = Formatted::<Email>::read(&serde_json::json!("a@b.com")).expect("reads");

    assert_eq!(value.as_str(), "a@b.com");
    assert_eq!(value.write(), serde_json::json!("a@b.com"));
    assert_eq!(value.clone().into_inner(), "a@b.com");
}

#[test]
fn a_formatted_value_is_not_read_from_a_non_string() {
    assert!(Formatted::<Email>::read(&serde_json::json!(1)).is_none());
}

#[test]
fn a_formatted_value_displays_as_its_string() {
    let value = Formatted::<Email>::read(&serde_json::json!("a@b.com")).unwrap();
    assert_eq!(value.to_string(), "a@b.com");
}

/// The debug form names the format, so a log line says what failed.
#[test]
fn the_debug_form_names_the_format() {
    let value = Formatted::<Email>::read(&serde_json::json!("a@b.com")).unwrap();
    let shown = format!("{value:?}");

    assert!(shown.contains("email"), "{shown}");
    assert!(shown.contains("a@b.com"), "{shown}");
}

#[test]
fn formatted_values_compare_and_sort_by_their_string() {
    let a = Formatted::<Email>::read(&serde_json::json!("a@b.com")).unwrap();
    let b = Formatted::<Email>::read(&serde_json::json!("b@b.com")).unwrap();

    assert_eq!(a, a.clone());
    assert_ne!(a, b);
    assert!(a < b);
}

#[test]
fn a_default_formatted_value_is_empty() {
    assert_eq!(Formatted::<Email>::default().as_str(), "");
}

#[test]
fn a_format_marker_carries_its_wire_name() {
    assert_eq!(Email::NAME, "email");
    assert_eq!(Date::NAME, "date");
    assert_eq!(Uuid::NAME, "uuid");
}

/// A format this crate does not ship is declared the same way.
#[test]
fn a_custom_format_declares_its_own_name() {
    struct EmployeeId;
    impl FormatName for EmployeeId {
        const NAME: &'static str = "employee-id";
    }

    assert_eq!(Formatted::<EmployeeId>::schema()["format"], "employee-id");
}
