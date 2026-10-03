//! `DocumentValidator`: the request envelope, and the pointers on failure.
//!
//! The pay-off this module exists for is that one malformed body comes back
//! as a complete error document pointing at every member that caused it — so
//! that is what these assert, rather than merely that it failed.
use ash_jsonapi::validation::DocumentValidator;
use ash_jsonapi::{Context, NoRelationships, Rel, Schema};

struct Todo;

impl Schema for Todo {
    type Relationship = Rel;

    fn name(&self) -> &str {
        "todo"
    }

    fn base_path(&self) -> &str {
        "/todos"
    }

    fn relationships(&self) -> &[Rel] {
        const RELS: &[Rel] = &[Rel::to_one("owner", "user"), Rel::to_many("tags", "tag")];
        RELS
    }
}

struct Note;

impl Schema for Note {
    type Relationship = NoRelationships;

    fn name(&self) -> &str {
        "note"
    }
    fn base_path(&self) -> &str {
        "/notes"
    }
    fn relationships(&self) -> &[NoRelationships] {
        &[]
    }
}

fn attributes() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "message": { "type": "string" },
            "done": { "type": "boolean" },
        },
        "required": ["message"],
    })
}

fn validator() -> DocumentValidator {
    DocumentValidator::builder(&Todo)
        .attributes(attributes())
        .build()
        .expect("the schema compiles")
}

fn ctx() -> Context {
    Context::new("c")
}

fn body(value: serde_json::Value) -> String {
    value.to_string()
}

#[test]
fn a_well_formed_document_passes() {
    let json = validator()
        .parse(
            &body(serde_json::json!({
                "data": { "type": "todo", "attributes": { "message": "buy milk" } }
            })),
            &ctx(),
        )
        .expect("valid");

    assert_eq!(json["data"]["attributes"]["message"], "buy milk");
}

#[test]
fn a_body_that_is_not_json_is_a_422_not_a_panic() {
    let errors = validator()
        .parse("{not json", &ctx())
        .expect_err("malformed");

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].status().as_u16(), 422);
    assert!(
        errors[0].to_object().detail.unwrap().contains("malformed"),
        "the failure should say what kind it was"
    );
}

#[test]
fn a_missing_required_attribute_is_refused() {
    let errors = validator()
        .parse(
            &body(serde_json::json!({ "data": { "type": "todo", "attributes": {} } })),
            &ctx(),
        )
        .expect_err("message is required");

    assert!(!errors.is_empty());
    assert_eq!(errors[0].status().as_u16(), 422);
}

#[test]
fn a_wrong_type_member_is_refused() {
    // The envelope's `type` is constrained to the resource's own name.
    let errors = validator()
        .parse(
            &body(serde_json::json!({
                "data": { "type": "user", "attributes": { "message": "hi" } }
            })),
            &ctx(),
        )
        .expect_err("wrong type");

    assert!(!errors.is_empty());
}

#[test]
fn a_missing_data_member_is_refused() {
    let errors = validator()
        .parse(&body(serde_json::json!({ "attributes": {} })), &ctx())
        .expect_err("no data");

    assert!(!errors.is_empty());
}

/// The instance location is already a JSON Pointer, which is what
/// `source.pointer` wants.
#[test]
fn a_failure_points_at_the_member_that_caused_it() {
    let errors = validator()
        .parse(
            &body(serde_json::json!({
                "data": { "type": "todo", "attributes": { "message": 42 } }
            })),
            &ctx(),
        )
        .expect_err("message is not a string");

    let pointer = errors[0]
        .to_object()
        .source
        .and_then(|source| source.pointer)
        .expect("a pointer");

    assert!(
        pointer.contains("message"),
        "the pointer should name the member: {pointer}"
    );
}

/// A client fixing a form wants the whole list, not the first problem.
#[test]
fn every_failure_is_reported_not_just_the_first() {
    let errors = validator()
        .parse(
            &body(serde_json::json!({
                "data": { "type": "user", "attributes": { "message": 42, "done": "yes" } }
            })),
            &ctx(),
        )
        .expect_err("several problems");

    assert!(
        errors.len() > 1,
        "expected several errors, got {}: {errors:?}",
        errors.len()
    );
}

#[test]
fn every_error_carries_the_correlation_id() {
    let ctx = Context::new("01H9K2QZ8V");
    let errors = validator()
        .parse(
            &body(serde_json::json!({ "data": { "type": "todo", "attributes": {} } })),
            &ctx,
        )
        .expect_err("invalid");

    for error in &errors {
        assert_eq!(error.to_object().id.as_deref(), Some("01H9K2QZ8V"));
    }
}

#[test]
fn check_takes_a_parsed_value_directly() {
    let value = serde_json::json!({
        "data": { "type": "todo", "attributes": { "message": "hi" } }
    });

    assert!(validator().check(&value, &ctx()).is_ok());
}

// --- Relationships -----------------------------------------------------

#[test]
fn a_declared_to_one_relationship_is_accepted() {
    let json = validator()
        .parse(
            &body(serde_json::json!({
                "data": {
                    "type": "todo",
                    "attributes": { "message": "hi" },
                    "relationships": { "owner": { "data": { "type": "user", "id": "u_1" } } }
                }
            })),
            &ctx(),
        )
        .expect("valid linkage");

    assert_eq!(json["data"]["relationships"]["owner"]["data"]["id"], "u_1");
}

/// The schema says which arity a relationship has, so the wrong shape is
/// refused rather than silently coerced.
#[test]
fn an_array_sent_for_a_to_one_relationship_is_refused() {
    let errors = validator()
        .parse(
            &body(serde_json::json!({
                "data": {
                    "type": "todo",
                    "attributes": { "message": "hi" },
                    "relationships": { "owner": { "data": [{ "type": "user", "id": "u_1" }] } }
                }
            })),
            &ctx(),
        )
        .expect_err("a to-one is not an array");

    assert!(!errors.is_empty());
}

#[test]
fn a_wrong_target_type_in_linkage_is_refused() {
    let errors = validator()
        .parse(
            &body(serde_json::json!({
                "data": {
                    "type": "todo",
                    "attributes": { "message": "hi" },
                    "relationships": { "owner": { "data": { "type": "todo", "id": "t_9" } } }
                }
            })),
            &ctx(),
        )
        .expect_err("the far side is a user");

    assert!(!errors.is_empty());
}

#[test]
fn a_to_many_relationship_takes_an_array() {
    validator()
        .parse(
            &body(serde_json::json!({
                "data": {
                    "type": "todo",
                    "attributes": { "message": "hi" },
                    "relationships": { "tags": { "data": [{ "type": "tag", "id": "g_1" }] } }
                }
            })),
            &ctx(),
        )
        .expect("valid to-many linkage");
}

// --- Builder options ---------------------------------------------------

/// For fields storage has but the wire format does not — the primary key,
/// and any foreign key carried as linkage.
#[test]
fn a_hidden_property_is_dropped_from_the_schema() {
    let validator = DocumentValidator::builder(&Todo)
        .attributes(serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" },
                "id": { "type": "string" },
            },
            "additionalProperties": false,
        }))
        .hide(["id"])
        .build()
        .expect("compiles");

    // `id` is no longer a declared property, so sending it is refused.
    let errors = validator
        .parse(
            &body(serde_json::json!({
                "data": { "type": "todo", "attributes": { "message": "hi", "id": "t_1" } }
            })),
            &ctx(),
        )
        .expect_err("id was hidden");

    assert!(!errors.is_empty());
}

#[test]
fn a_declared_format_is_asserted_by_default() {
    let validator = DocumentValidator::builder(&Note)
        .attributes(serde_json::json!({
            "type": "object",
            "properties": { "email": { "type": "string", "format": "email" } },
        }))
        .build()
        .expect("compiles");

    let errors = validator
        .parse(
            &body(serde_json::json!({
                "data": { "type": "note", "attributes": { "email": "not-an-address" } }
            })),
            &ctx(),
        )
        .expect_err("a declared format is a rule, not a decoration");

    assert!(!errors.is_empty());
}

#[test]
fn annotate_formats_turns_the_assertion_off() {
    let validator = DocumentValidator::builder(&Note)
        .attributes(serde_json::json!({
            "type": "object",
            "properties": { "email": { "type": "string", "format": "email" } },
        }))
        .annotate_formats()
        .build()
        .expect("compiles");

    validator
        .parse(
            &body(serde_json::json!({
                "data": { "type": "note", "attributes": { "email": "not-an-address" } }
            })),
            &ctx(),
        )
        .expect("format is only an annotation here");
}

#[test]
fn a_custom_format_check_is_applied() {
    let validator = DocumentValidator::builder(&Note)
        .attributes(serde_json::json!({
            "type": "object",
            "properties": { "badge": { "type": "string", "format": "employee-id" } },
        }))
        .format("employee-id", |value| {
            value.strip_prefix("E-").is_some_and(|n| n.len() == 6)
        })
        .build()
        .expect("compiles");

    validator
        .parse(
            &body(serde_json::json!({
                "data": { "type": "note", "attributes": { "badge": "E-123456" } }
            })),
            &ctx(),
        )
        .expect("matches the check");

    let errors = validator
        .parse(
            &body(serde_json::json!({
                "data": { "type": "note", "attributes": { "badge": "nope" } }
            })),
            &ctx(),
        )
        .expect_err("does not match the check");

    assert!(!errors.is_empty());
}

#[test]
fn a_resource_with_no_relationships_still_builds() {
    DocumentValidator::builder(&Note)
        .attributes(serde_json::json!({ "type": "object" }))
        .build()
        .expect("a flat resource compiles");
}
