//! Filenames and media types come from clients, so both are attacker input.
use ash_jsonapi::response::disposition;

#[test]
fn a_quote_cannot_break_out_of_the_header() {
    // Without escaping this would end the quoted string and inject a param.
    let d = disposition(false, Some(r#"evil".txt"#));
    assert!(d.starts_with("attachment; filename=\""), "{d}");
    // The quote is escaped, not raw.
    assert!(d.contains(r#"\""#), "{d}");
    // And it never parses into two header values.
    assert!(!d.contains('\n') && !d.contains('\r'), "{d}");
}

#[test]
fn control_characters_are_removed() {
    let d = disposition(false, Some("a\r\nSet-Cookie: x=1"));
    assert!(
        !d.contains('\r') && !d.contains('\n'),
        "CRLF injection: {d}"
    );
    // The header must still parse.
    assert!(d.parse::<axum::http::HeaderValue>().is_ok(), "{d}");
}

#[test]
fn unicode_survives_via_the_encoded_form() {
    let d = disposition(false, Some("naïve fötö.png"));
    // ASCII fallback has no raw non-ASCII…
    assert!(d.is_ascii(), "{d}");
    // …and the real name is carried percent-encoded.
    assert!(d.contains("filename*=UTF-8''"), "{d}");
    assert!(d.contains("%C3%AF"), "ï should be percent-encoded: {d}");
}

#[test]
fn inline_and_attachment_are_distinct() {
    assert!(disposition(true, None).starts_with("inline"));
    assert!(disposition(false, None).starts_with("attachment"));
    assert_eq!(disposition(false, None), "attachment");
}

#[test]
fn a_path_traversal_name_is_defanged_into_one_header_value() {
    let d = disposition(false, Some("../../etc/passwd"));
    assert!(d.parse::<axum::http::HeaderValue>().is_ok(), "{d}");
    // Slashes are legal in a quoted string; what matters is that the value
    // stays one header and the client treats it as a name, not a path.
    assert!(d.contains("filename*=UTF-8''"), "{d}");
}
