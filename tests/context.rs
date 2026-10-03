//! The per-request `Context`: what it carries, and its copy-on-write sharing.
use ash_jsonapi::Context;

#[test]
fn a_correlation_id_is_always_present() {
    let ctx = Context::new("01H9K2QZ8V");
    assert_eq!(ctx.correlation_id(), "01H9K2QZ8V");
}

#[test]
fn a_fresh_context_carries_nothing_else() {
    let ctx = Context::new("c");

    assert_eq!(ctx.principal(), None);
    assert_eq!(ctx.tenant(), None);
    assert_eq!(ctx.locale(), None);
    assert!(ctx.metadata().is_empty());
}

#[test]
fn the_builders_bind_what_they_name() {
    let ctx = Context::new("c")
        .with_principal("alice@example.com")
        .with_tenant("acme");

    assert_eq!(ctx.principal(), Some("alice@example.com"));
    assert_eq!(ctx.tenant(), Some("acme"));
}

#[test]
fn metadata_accumulates_and_reads_back() {
    let ctx = Context::new("c")
        .with_metadata("attempt", 2)
        .with_metadata("source", "webhook");

    assert_eq!(ctx.get("attempt").unwrap(), 2);
    assert_eq!(ctx.get("source").unwrap(), "webhook");
    assert_eq!(ctx.metadata().len(), 2);
    assert!(ctx.get("nonesuch").is_none());
}

#[test]
fn a_locale_is_recorded_when_one_was_negotiated() {
    let locale = ash_jsonapi::Locale::negotiate("de-DE").unwrap();
    let ctx = Context::new("c").with_locale(locale);

    assert_eq!(ctx.locale().unwrap().as_str(), "de-DE");
}

/// A `Context` is cloned per request and handed to nested calls, so an edit
/// on one copy must not be visible through another.
#[test]
fn a_shared_context_is_not_mutated_by_a_later_edit() {
    let original = Context::new("c").with_principal("alice");
    let extended = original.clone().with_tenant("acme");

    assert_eq!(original.tenant(), None, "the original is untouched");
    assert_eq!(extended.tenant(), Some("acme"));
    assert_eq!(
        extended.principal(),
        Some("alice"),
        "and what it already carried survives"
    );
}

#[test]
fn rebinding_a_field_replaces_it() {
    let ctx = Context::new("c")
        .with_principal("alice")
        .with_principal("bob");

    assert_eq!(ctx.principal(), Some("bob"));
}

// --- Typed extensions --------------------------------------------------

#[derive(Debug, PartialEq)]
struct Actor(&'static str);

#[derive(Debug, PartialEq)]
struct Flags(u8);

#[test]
fn a_typed_value_reads_back_by_its_type() {
    let ctx = Context::new("c").with(Actor("alice"));

    assert_eq!(ctx.get_ext::<Actor>(), Some(&Actor("alice")));
    assert_eq!(ctx.extract::<Actor>().unwrap(), &Actor("alice"));
}

#[test]
fn different_types_coexist() {
    let ctx = Context::new("c").with(Actor("alice")).with(Flags(3));

    assert_eq!(ctx.get_ext::<Actor>(), Some(&Actor("alice")));
    assert_eq!(ctx.get_ext::<Flags>(), Some(&Flags(3)));
}

#[test]
fn a_second_value_of_one_type_replaces_the_first() {
    let ctx = Context::new("c").with(Actor("alice")).with(Actor("bob"));

    assert_eq!(ctx.get_ext::<Actor>(), Some(&Actor("bob")));
}

#[test]
fn an_absent_extension_is_none() {
    let ctx = Context::new("c");
    assert_eq!(ctx.get_ext::<Actor>(), None);
}

/// A missing extension is a wiring bug, not a bad request — so the client is
/// told nothing beyond the correlation id.
#[test]
fn extracting_a_missing_extension_is_a_sanitized_500() {
    let ctx = Context::new("01H9K2QZ8V");
    let err = ctx.extract::<Actor>().expect_err("a wiring bug");

    assert_eq!(err.status().as_u16(), 500);

    let object = err.to_object();
    assert_eq!(object.id.as_deref(), Some("01H9K2QZ8V"));
    assert_eq!(
        object.detail.as_deref(),
        Some("An internal error occurred."),
        "the type name must not reach the client"
    );
}

#[test]
fn extensions_survive_a_clone() {
    let ctx = Context::new("c").with(Actor("alice"));
    let cloned = ctx.clone();

    assert_eq!(cloned.get_ext::<Actor>(), Some(&Actor("alice")));
}

#[test]
fn an_extension_added_to_a_clone_is_not_visible_on_the_original() {
    let original = Context::new("c");
    let extended = original.clone().with(Actor("alice"));

    assert_eq!(original.get_ext::<Actor>(), None);
    assert_eq!(extended.get_ext::<Actor>(), Some(&Actor("alice")));
}
