//! `Accept-Language` negotiation, and the translation keys on an error.
//!
//! `Locale::negotiate` runs on every request through `layer()`, and the tag
//! it returns is echoed into a response body — so it is both hot and a place
//! where client-supplied input reaches output.
use ash_jsonapi::i18n::{Locale, Translation};

#[test]
fn the_highest_q_wins_over_header_order() {
    let best = Locale::negotiate("en;q=0.5, fr;q=0.9").expect("a match");
    assert_eq!(best.as_str(), "fr");
}

#[test]
fn a_missing_q_means_one() {
    // `de-DE` has no `q`, so it outranks the explicit 0.9 behind it.
    let best = Locale::negotiate("de-DE,de;q=0.9,en;q=0.8").expect("a match");
    assert_eq!(best.as_str(), "de-DE");
}

#[test]
fn an_equal_q_keeps_the_earlier_entry() {
    // The RFC leaves the tie-break to the server; this crate takes position.
    let best = Locale::negotiate("fr;q=0.5, de;q=0.5").expect("a match");
    assert_eq!(best.as_str(), "fr");
}

#[test]
fn q_zero_is_never_acceptable() {
    // `q=0` is a client saying it will *not* take that language.
    assert!(Locale::negotiate("en;q=0").is_none());

    // And it loses to anything that is acceptable, wherever it sits.
    let best = Locale::negotiate("en;q=0, fr;q=0.1").expect("a match");
    assert_eq!(best.as_str(), "fr");
}

#[test]
fn the_wildcard_names_no_locale() {
    assert!(Locale::negotiate("*").is_none());
    // And it is skipped rather than ending the search.
    let best = Locale::negotiate("*, es").expect("a match");
    assert_eq!(best.as_str(), "es");
}

#[test]
fn an_empty_or_absent_header_negotiates_nothing() {
    assert!(Locale::negotiate("").is_none());
    assert!(Locale::negotiate(",,").is_none());
    assert!(Locale::negotiate("   ").is_none());
}

/// The tag is echoed into a response body, so it is filtered rather than
/// reflected verbatim.
#[test]
fn a_tag_that_is_not_shaped_like_one_is_refused() {
    assert!(Locale::negotiate("en_US; DROP TABLE").is_none());
    assert!(Locale::negotiate("<script>").is_none());
    assert!(Locale::negotiate("en US").is_none());
    // Over the 35-character cap.
    assert!(Locale::negotiate(&"a".repeat(36)).is_none());
    // At the cap it is still fine.
    assert!(Locale::negotiate(&"a".repeat(35)).is_some());
}

#[test]
fn an_unparsable_q_falls_back_to_one() {
    // `q=high` is not a number; the entry stays acceptable at the default.
    let best = Locale::negotiate("en;q=high").expect("a match");
    assert_eq!(best.as_str(), "en");
}

#[test]
fn other_parameters_do_not_defeat_the_parse() {
    let best = Locale::negotiate("en;charset=utf-8;q=0.4, de;q=0.8").expect("a match");
    assert_eq!(best.as_str(), "de");
}

#[test]
fn a_locale_displays_as_its_tag() {
    let locale = Locale::negotiate("pt-BR").expect("a match");
    assert_eq!(locale.to_string(), "pt-BR");
    assert_eq!(locale.as_str(), "pt-BR");
}

#[test]
fn a_translation_carries_its_key_and_params() {
    let t = Translation::new("todo.name.too_long")
        .param("max", 80)
        .param("field", "name");

    assert_eq!(t.key, "todo.name.too_long");
    assert_eq!(t.params["max"], 80);
    assert_eq!(t.params["field"], "name");
}

#[test]
fn empty_params_are_omitted_from_the_wire() {
    let json = serde_json::to_value(Translation::new("todo.not_found")).unwrap();

    assert_eq!(json["key"], "todo.not_found");
    assert!(
        json.get("params").is_none(),
        "a message with no placeholders carries no empty object: {json}"
    );
}

#[test]
fn a_translation_round_trips() {
    let t = Translation::new("a.b").param("n", 1);
    let back: Translation = serde_json::from_value(serde_json::to_value(&t).unwrap()).unwrap();
    assert_eq!(t, back);
}
