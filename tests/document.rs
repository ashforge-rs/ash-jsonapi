//! The JSON:API wire types: what they serialize to, and what round-trips.
//!
//! `PrimaryData` and `Linkage` are `#[serde(untagged)]`, so the shape on the
//! wire is the only thing distinguishing a single resource from a collection,
//! or a null to-one from an absent member. That makes these assertions the
//! spec conformance the crate rests on.
use ash_jsonapi::document::{Linkage, PrimaryData, Relationship, ResourceIdentifier};
use ash_jsonapi::{Document, ErrorObject, Links, ResourceObject};

fn todo() -> ResourceObject {
    ResourceObject::new("todo", "t_1").attr("message", "buy milk")
}

#[test]
fn a_single_document_carries_an_object_not_an_array() {
    let json = serde_json::to_value(Document::single(todo())).unwrap();

    assert!(json["data"].is_object(), "{json}");
    assert_eq!(json["data"]["type"], "todo");
    assert_eq!(json["data"]["id"], "t_1");
    assert!(json.get("errors").is_none(), "never both members");
}

#[test]
fn a_collection_document_carries_an_array() {
    let json = serde_json::to_value(Document::collection(vec![todo()])).unwrap();

    assert!(json["data"].is_array(), "{json}");
    assert_eq!(json["data"][0]["id"], "t_1");
}

#[test]
fn an_empty_collection_is_an_empty_array_not_absent() {
    let json = serde_json::to_value(Document::collection(vec![])).unwrap();

    assert_eq!(json["data"], serde_json::json!([]));
}

#[test]
fn an_error_document_carries_no_data_member() {
    let json = serde_json::to_value(Document::errors(vec![ErrorObject::default()])).unwrap();

    assert!(json["errors"].is_array());
    assert!(
        json.get("data").is_none(),
        "a document has data or errors, never both: {json}"
    );
}

#[test]
fn absent_members_are_omitted_entirely() {
    let json = serde_json::to_value(Document::single(todo())).unwrap();

    for absent in ["errors", "included", "links", "meta"] {
        assert!(json.get(absent).is_none(), "`{absent}` should be omitted");
    }
}

#[test]
fn meta_members_accumulate_rather_than_replace() {
    let document = Document::collection(vec![])
        .with_meta("total", 42)
        .with_meta("lang", "de-DE");

    let json = serde_json::to_value(document).unwrap();
    assert_eq!(json["meta"]["total"], 42, "a paged listing keeps its total");
    assert_eq!(json["meta"]["lang"], "de-DE");
}

#[test]
fn an_untagged_document_round_trips_through_both_shapes() {
    for document in [
        Document::single(todo()),
        Document::collection(vec![todo(), todo()]),
    ] {
        let json = serde_json::to_string(&document).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(document, back, "untagged shapes must survive a round trip");
    }
}

#[test]
fn a_single_resource_deserializes_as_single() {
    let back: Document = serde_json::from_str(r#"{"data":{"type":"todo","id":"t_1"}}"#).unwrap();

    match back.data.expect("data") {
        PrimaryData::Single(resource) => assert_eq!(resource.id.as_deref(), Some("t_1")),
        PrimaryData::Collection(_) => panic!("an object must not parse as a collection"),
    }
}

#[test]
fn an_array_deserializes_as_a_collection() {
    let back: Document = serde_json::from_str(r#"{"data":[{"type":"todo","id":"t_1"}]}"#).unwrap();

    match back.data.expect("data") {
        PrimaryData::Collection(rows) => assert_eq!(rows.len(), 1),
        PrimaryData::Single(_) => panic!("an array must not parse as a single resource"),
    }
}

#[test]
fn an_unrequested_attribute_is_absent_never_null() {
    let json = serde_json::to_value(ResourceObject::new("todo", "t_1")).unwrap();

    assert!(
        json.get("attributes").is_none(),
        "an empty attribute map is omitted: {json}"
    );
    assert!(json.get("relationships").is_none());
}

#[test]
fn a_client_generated_create_may_carry_no_id() {
    let mut resource = ResourceObject::new("todo", "t_1");
    resource.id = None;

    let json = serde_json::to_value(resource).unwrap();
    assert_eq!(json["type"], "todo");
    assert!(json.get("id").is_none(), "{json}");
}

#[test]
fn an_empty_to_one_is_null_and_an_absent_one_is_missing() {
    let null = serde_json::to_value(Relationship::empty_to_one()).unwrap();
    assert!(null["data"].is_null(), "an explicit null: {null}");

    let absent = serde_json::to_value(Relationship {
        data: None,
        links: None,
    })
    .unwrap();
    assert!(
        absent.get("data").is_none(),
        "absent is not the same as null: {absent}"
    );
}

#[test]
fn to_one_and_to_many_linkage_have_distinct_shapes() {
    let one =
        serde_json::to_value(Relationship::to_one(ResourceIdentifier::new("user", "u_1"))).unwrap();
    assert!(one["data"].is_object(), "{one}");

    let many = serde_json::to_value(Relationship::to_many(vec![ResourceIdentifier::new(
        "tag", "g_1",
    )]))
    .unwrap();
    assert!(many["data"].is_array(), "{many}");
}

#[test]
fn linkage_round_trips_through_all_three_shapes() {
    for linkage in [
        Linkage::One(None),
        Linkage::One(Some(ResourceIdentifier::new("user", "u_1"))),
        Linkage::Many(vec![ResourceIdentifier::new("tag", "g_1")]),
    ] {
        let json = serde_json::to_string(&linkage).unwrap();
        let back: Linkage = serde_json::from_str(&json).unwrap();
        assert_eq!(linkage, back, "{json}");
    }
}

#[test]
fn the_self_link_is_spelled_self_on_the_wire() {
    let json = serde_json::to_value(Links::this("/todos/t_1")).unwrap();

    assert_eq!(json["self"], "/todos/t_1");
    assert!(
        json.get("this").is_none(),
        "`this` is the Rust name only: {json}"
    );
}

#[test]
fn only_the_links_that_were_set_are_sent() {
    let json = serde_json::to_value(Links::this("/todos").next("/todos?page=2")).unwrap();

    assert_eq!(json["next"], "/todos?page=2");
    for absent in ["first", "prev", "last", "related"] {
        assert!(json.get(absent).is_none(), "`{absent}` should be omitted");
    }
}

#[test]
fn a_resource_identifier_is_just_type_and_id() {
    let json = serde_json::to_value(ResourceIdentifier::new("user", "u_1")).unwrap();

    assert_eq!(json, serde_json::json!({ "type": "user", "id": "u_1" }));
}

#[test]
fn included_resources_ride_alongside_the_primary_data() {
    let document = Document::single(todo()).with_included(vec![ResourceObject::new("user", "u_1")]);
    let json = serde_json::to_value(document).unwrap();

    assert_eq!(json["included"][0]["type"], "user");
}
