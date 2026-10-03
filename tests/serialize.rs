//! `Schema` + data → `ResourceObject`, with no ash-domain in the tree.
//!
//! This is the standalone path the crate claims to support, so exercising it
//! here is what keeps that claim honest rather than merely documented.
use ash_jsonapi::document::Linkage;
use ash_jsonapi::{NoRelationships, Rel, Schema, Serializer};

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

/// A resource that declares nothing on its far side.
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

#[test]
fn a_record_carries_its_type_id_and_self_link() {
    let object = Serializer::new(&Todo)
        .record("t_1")
        .attr("message", "buy milk")
        .build();

    assert_eq!(object.kind, "todo");
    assert_eq!(object.id.as_deref(), Some("t_1"));
    assert_eq!(object.attributes["message"], "buy milk");
    assert_eq!(object.links.unwrap().this.unwrap(), "/todos/t_1");
}

#[test]
fn a_to_one_relationship_is_a_single_linkage() {
    let object = Serializer::new(&Todo)
        .record("t_1")
        .rel("owner", "u_1")
        .build();

    let owner = &object.relationships["owner"];
    match owner.data.as_ref().expect("linkage") {
        Linkage::One(Some(id)) => {
            assert_eq!(id.kind, "user", "the target type comes from the schema");
            assert_eq!(id.id, "u_1");
        }
        other => panic!("a to-one must not serialize as an array: {other:?}"),
    }
}

#[test]
fn a_to_many_relationship_is_an_array() {
    let object = Serializer::new(&Todo)
        .record("t_1")
        .rel_many("tags", ["g_1".to_string(), "g_2".to_string()])
        .build();

    match object.relationships["tags"].data.as_ref().expect("linkage") {
        Linkage::Many(ids) => {
            assert_eq!(ids.len(), 2);
            assert_eq!(ids[0].kind, "tag");
        }
        other => panic!("a to-many must be an array: {other:?}"),
    }
}

/// Arity follows the schema, not the data — the property the module exists
/// to guarantee.
#[test]
fn an_empty_to_one_is_null_not_an_empty_array() {
    let object = Serializer::new(&Todo)
        .record("t_1")
        .rel_many("owner", [])
        .build();

    match object.relationships["owner"]
        .data
        .as_ref()
        .expect("linkage")
    {
        Linkage::One(None) => {}
        other => panic!("an unset to-one is an explicit null: {other:?}"),
    }

    // And it renders as `null`, not `[]`.
    let json = serde_json::to_value(&object).unwrap();
    assert!(json["relationships"]["owner"]["data"].is_null());
}

#[test]
fn an_empty_to_many_stays_an_empty_array() {
    let object = Serializer::new(&Todo)
        .record("t_1")
        .rel_many("tags", [])
        .build();

    let json = serde_json::to_value(&object).unwrap();
    assert_eq!(
        json["relationships"]["tags"]["data"],
        serde_json::json!([]),
        "a to-many with nothing on the far side is empty, not null"
    );
}

#[test]
fn an_undeclared_relationship_is_dropped_rather_than_guessed_at() {
    let object = Serializer::new(&Todo)
        .record("t_1")
        .rel("nonesuch", "x_1")
        .build();

    assert!(
        !object.relationships.contains_key("nonesuch"),
        "the schema is the authority on what exists"
    );
}

#[test]
fn a_relationship_carries_both_links() {
    let object = Serializer::new(&Todo)
        .record("t_1")
        .rel("owner", "u_1")
        .build();

    let links = object.relationships["owner"].links.as_ref().expect("links");
    assert_eq!(
        links.this.as_deref(),
        Some("/todos/t_1/relationships/owner")
    );
    assert_eq!(links.related.as_deref(), Some("/todos/t_1/owner"));
}

#[test]
fn a_resource_with_no_relationships_needs_no_relationship_type() {
    let object = Serializer::new(&Note)
        .record("n_1")
        .attr("body", "hi")
        .build();

    assert_eq!(object.kind, "note");
    assert!(object.relationships.is_empty());
    assert_eq!(object.links.unwrap().this.unwrap(), "/notes/n_1");
}

#[test]
fn a_trailing_slash_on_the_base_path_does_not_double_up() {
    struct Slashed;
    impl Schema for Slashed {
        type Relationship = NoRelationships;
        fn name(&self) -> &str {
            "note"
        }
        fn base_path(&self) -> &str {
            "/notes/"
        }
        fn relationships(&self) -> &[NoRelationships] {
            &[]
        }
    }

    assert_eq!(Serializer::new(&Slashed).self_link("n_1"), "/notes/n_1");
}

#[test]
fn attrs_sets_several_at_once() {
    let object = Serializer::new(&Todo)
        .record("t_1")
        .attrs([
            ("message".to_string(), serde_json::json!("buy milk")),
            ("done".to_string(), serde_json::json!(false)),
        ])
        .build();

    assert_eq!(object.attributes["message"], "buy milk");
    assert_eq!(object.attributes["done"], false);
}

#[test]
fn the_bulk_path_matches_the_fluent_one() {
    use std::collections::BTreeMap;

    let fluent = Serializer::new(&Todo)
        .record("t_1")
        .attr("message", "buy milk")
        .rel("owner", "u_1")
        .build();

    let attributes = BTreeMap::from([("message".to_string(), serde_json::json!("buy milk"))]);
    let related = BTreeMap::from([("owner".to_string(), vec!["u_1".to_string()])]);
    let bulk = Serializer::new(&Todo).serialize("t_1", attributes, &related);

    assert_eq!(fluent, bulk);
}
