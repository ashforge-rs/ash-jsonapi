//! The `ash-domain` seam: schema → wire format, and domain errors → statuses.
//!
//! Deliberately narrow. Everything here is a pure conversion that needs no
//! running domain, no data layer and no policies — which is what makes it
//! worth having as a first pass: the translation layer is where a JSON:API
//! response is actually shaped, and it can be checked in isolation.
//!
//! `ash-domain` is an optional dependency rather than a dev-dependency, so
//! its types are reached through the crate's own re-export.
use ash_jsonapi::__ash_domain::value::{Record, Value};
use ash_jsonapi::domain::{Mounted, error_from};
use ash_jsonapi::serialize::{RelationshipDef, Schema};
use ash_jsonapi::{Context, resource_schema};

fn ctx() -> Context {
    Context::new("01H9K2QZ8V")
}

// --- Schema → the serializer's view ------------------------------------

#[test]
fn a_mounted_schema_reports_its_name_and_path() {
    let schema = resource_schema!("todo" { message: String });
    let mounted = Mounted::new(&schema, "/api/v1/todos");

    assert_eq!(mounted.name(), "todo");
    assert_eq!(mounted.base_path(), "/api/v1/todos");
    assert_eq!(mounted.self_link("t_1"), "/api/v1/todos/t_1");
}

/// Cardinality reaches this crate as a string, and only the arity matters to
/// JSON:API — a to-one is a linkage object, a to-many an array.
#[test]
fn cardinality_decides_arity() {
    let schema = resource_schema! {
        "todo" {
            message: String,
            owner -> "user",
            tags -> ["tag"],
        }
    };

    let owner = &schema.relationships[0];
    assert_eq!(owner.name(), "owner");
    assert_eq!(owner.target(), "user", "the far side comes from the schema");
    assert!(!owner.is_to_many(), "`belongs_to` is a to-one");

    let tags = &schema.relationships[1];
    assert_eq!(tags.target(), "tag");
    assert!(tags.is_to_many(), "`has_many` is a to-many");
}

// --- Record → ResourceObject -------------------------------------------

#[test]
fn a_domain_record_becomes_a_resource_object() {
    let schema = resource_schema!("todo" { message: String, done: Boolean });
    let mounted = Mounted::new(&schema, "/todos");

    let mut record = Record::new();
    record.insert("id", Value::Str("t_1".into()));
    record.insert("message", Value::Str("buy milk".into()));
    record.insert("done", Value::Bool(false));

    let object = mounted.record_from(&record).build();

    assert_eq!(object.kind, "todo");
    assert_eq!(object.id.as_deref(), Some("t_1"));
    assert_eq!(object.attributes["message"], "buy milk");
    assert_eq!(object.attributes["done"], false);
    assert_eq!(object.links.unwrap().this.unwrap(), "/todos/t_1");
}

/// The primary key is the envelope `id`, so it must not also sit in
/// `attributes` — JSON:API carries it in exactly one place.
#[test]
fn the_primary_key_is_not_repeated_as_an_attribute() {
    let schema = resource_schema!("todo" { message: String });
    let mounted = Mounted::new(&schema, "/todos");

    let mut record = Record::new();
    record.insert("id", Value::Str("t_1".into()));
    record.insert("message", Value::Str("buy milk".into()));

    let object = mounted.record_from(&record).build();

    assert_eq!(object.id.as_deref(), Some("t_1"));
    assert!(
        !object.attributes.contains_key("id"),
        "the id belongs in the envelope, not beside the data: {:?}",
        object.attributes
    );
}

/// A foreign key belongs in linkage, not in `attributes`.
#[test]
fn a_relationships_source_attribute_is_dropped() {
    // `resource_schema!` gives every relationship `id` as its source
    // attribute, so declare the relationship on a resource whose own key is
    // distinct enough to see the filtering work.
    let schema = resource_schema! {
        "todo" {
            message: String,
            owner -> "user",
        }
    };
    let mounted = Mounted::new(&schema, "/todos");

    let mut record = Record::new();
    record.insert("id", Value::Str("t_1".into()));
    record.insert("message", Value::Str("buy milk".into()));

    let object = mounted.record_from(&record).build();

    assert_eq!(object.attributes.len(), 1, "{:?}", object.attributes);
    assert!(object.attributes.contains_key("message"));
}

#[test]
fn a_non_string_primary_key_still_becomes_a_string_id() {
    let schema = resource_schema!("todo" { message: String });
    let mounted = Mounted::new(&schema, "/todos");

    let mut record = Record::new();
    record.insert("id", Value::Int(42));
    record.insert("message", Value::Str("buy milk".into()));

    let object = mounted.record_from(&record).build();
    assert_eq!(object.id.as_deref(), Some("42"));
}

#[test]
fn relationships_are_added_on_top_of_a_record() {
    let schema = resource_schema! {
        "todo" {
            message: String,
            owner -> "user",
        }
    };
    let mounted = Mounted::new(&schema, "/todos");

    let mut record = Record::new();
    record.insert("id", Value::Str("t_1".into()));
    record.insert("message", Value::Str("buy milk".into()));

    let object = mounted.record_from(&record).rel("owner", "u_1").build();

    let linkage = object.relationships["owner"]
        .data
        .as_ref()
        .expect("linkage");
    assert_eq!(
        serde_json::to_value(linkage).unwrap(),
        serde_json::json!({ "type": "user", "id": "u_1" })
    );
}

// --- Value → JSON ------------------------------------------------------

/// `Value`'s own `Serialize` is externally tagged, so a bare `to_value` would
/// send `{"Str":"buy milk"}`. A JSON:API attribute is the bare value.
#[test]
fn every_value_renders_as_a_bare_json_value() {
    let schema = resource_schema!("thing" { message: String });
    let mounted = Mounted::new(&schema, "/things");

    let mut record = Record::new();
    record.insert("id", Value::Str("x".into()));
    record.insert("text", Value::Str("hi".into()));
    record.insert("flag", Value::Bool(true));
    record.insert("count", Value::Int(7));
    record.insert("ratio", Value::Float(1.5));
    record.insert("nothing", Value::Null);
    record.insert("when", Value::Timestamp(1_700_000_000_000));

    let attributes = mounted.record_from(&record).build().attributes;

    assert_eq!(attributes["text"], "hi");
    assert_eq!(attributes["flag"], true);
    assert_eq!(attributes["count"], 7);
    assert_eq!(attributes["ratio"], 1.5);
    assert!(attributes["nothing"].is_null());
    assert_eq!(
        attributes["when"], 1_700_000_000_000i64,
        "a timestamp is milliseconds since the epoch, as the domain stores it"
    );
}

#[test]
fn a_map_and_a_list_render_structurally() {
    let schema = resource_schema!("thing" { message: String });
    let mounted = Mounted::new(&schema, "/things");

    let mut record = Record::new();
    record.insert("id", Value::Str("x".into()));
    record.insert(
        "address",
        Value::Map(
            [("city".to_string(), Value::Str("Athens".into()))]
                .into_iter()
                .collect(),
        ),
    );
    record.insert("scores", Value::List(vec![Value::Int(1), Value::Int(2)]));

    let attributes = mounted.record_from(&record).build().attributes;

    assert_eq!(
        attributes["address"],
        serde_json::json!({ "city": "Athens" })
    );
    assert_eq!(attributes["scores"], serde_json::json!([1, 2]));
}

/// Bytes have no JSON form, and base64 is the caller's decision.
#[test]
fn bytes_render_as_null_rather_than_a_guess() {
    let schema = resource_schema!("thing" { message: String });
    let mounted = Mounted::new(&schema, "/things");

    let mut record = Record::new();
    record.insert("id", Value::Str("x".into()));
    record.insert("blob", Value::Bytes(vec![1, 2, 3].into()));

    let attributes = mounted.record_from(&record).build().attributes;
    assert!(attributes["blob"].is_null());
}

// --- Domain errors → JSON:API statuses ---------------------------------

#[test]
fn a_denial_is_a_403_not_a_500() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(&Error::Forbidden("policy said no".into()), &ctx());

    assert_eq!(err.status().as_u16(), 403);
    assert_eq!(err.to_object().code, "forbidden");
}

/// A policy *outage* is not a denial: the action failed closed, but the
/// caller may retry.
#[test]
fn a_policy_outage_is_distinct_from_a_denial() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(&Error::PolicyError("backend timed out".into()), &ctx());

    assert_eq!(err.status().as_u16(), 503);
    assert_eq!(err.to_object().code, "policy_unavailable");
    assert!(err.code.is_retriable());
}

#[test]
fn a_missing_record_is_a_404() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(&Error::NotFound("no such todo".into()), &ctx());
    assert_eq!(err.status().as_u16(), 404);
}

#[test]
fn a_missing_tenant_is_the_clients_mistake() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(&Error::MissingTenant("todo".into()), &ctx());
    assert_eq!(err.status().as_u16(), 400);
}

#[test]
fn an_invalid_field_becomes_a_pointer_into_the_document() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(
        &Error::Invalid {
            field: Some("address.city".into()),
            message: "must not be empty".into(),
        },
        &ctx(),
    );

    assert_eq!(err.status().as_u16(), 422);

    let object = err.to_object();
    assert_eq!(
        object.source.unwrap().pointer.unwrap(),
        "/data/attributes/address/city",
        "a dotted attribute path nests, as it does elsewhere in the crate"
    );
    assert_eq!(object.detail.as_deref(), Some("must not be empty"));
}

#[test]
fn an_invalid_with_no_field_points_at_the_document() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(
        &Error::Invalid {
            field: None,
            message: "one of `from`/`to` is required".into(),
        },
        &ctx(),
    );

    assert_eq!(err.to_object().source.unwrap().pointer.unwrap(), "/data");
}

/// The two `500`s stay opaque: a data-layer message can carry a connection
/// string or a fragment of a query.
#[test]
fn a_data_layer_failure_never_reaches_the_client() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(
        &Error::DataLayer {
            message: "postgres://user:hunter2@db.internal/app refused".into(),
            source: None,
        },
        &ctx(),
    );

    assert_eq!(err.status().as_u16(), 500);

    let object = err.to_object();
    assert_eq!(
        object.detail.as_deref(),
        Some("An internal error occurred.")
    );
    assert!(
        !format!("{object:?}").contains("hunter2"),
        "the credential must not survive anywhere in the object"
    );
}

/// A resource the router offers but the domain does not register is a wiring
/// bug, not something a client can act on.
#[test]
fn an_unknown_resource_is_a_sanitized_500() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(&Error::UnknownResource("widget".into()), &ctx());

    assert_eq!(err.status().as_u16(), 500);
    assert_eq!(
        err.to_object().detail.as_deref(),
        Some("An internal error occurred."),
        "the name of an unregistered resource is not the client's business"
    );
}

#[test]
fn an_unknown_action_is_a_sanitized_500() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(
        &Error::UnknownAction {
            resource: "todo".into(),
            action: "archive".into(),
        },
        &ctx(),
    );

    assert_eq!(err.status().as_u16(), 500);
}

#[test]
fn a_serialization_failure_is_a_sanitized_500() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(&Error::Serialization("bad column".into()), &ctx());
    assert_eq!(err.status().as_u16(), 500);
    assert_eq!(
        err.to_object().detail.as_deref(),
        Some("An internal error occurred.")
    );
}

#[test]
fn every_converted_error_carries_the_correlation_id() {
    use ash_jsonapi::__ash_domain::Error;

    let err = error_from(&Error::NotFound("gone".into()), &ctx());

    assert_eq!(
        err.to_object().id.as_deref(),
        Some("01H9K2QZ8V"),
        "this is what ties the response back to the audit log"
    );
}
