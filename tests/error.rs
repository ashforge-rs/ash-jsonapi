//! `JsonApiError`: the status each code carries, and what reaches the client.
//!
//! The sanitization rule is the one that matters most here — a data-layer
//! message can carry a connection string, and this is what keeps it out of a
//! response body.
use ash_jsonapi::error::INTERNAL_DETAIL;
use ash_jsonapi::{Context, ErrorCode, JsonApiError};

fn ctx() -> Context {
    Context::new("01H9K2QZ8V")
}

#[test]
fn every_code_maps_to_its_status() {
    for (code, status) in [
        (ErrorCode::NotFound, 404),
        (ErrorCode::Forbidden, 403),
        (ErrorCode::PolicyUnavailable, 503),
        (ErrorCode::MissingTenant, 400),
        (ErrorCode::InvalidAttribute, 422),
        (ErrorCode::InvalidParameter, 400),
        (ErrorCode::Conflict, 409),
        (ErrorCode::PreconditionFailed, 412),
        (ErrorCode::LockContention, 503),
        (ErrorCode::Closing, 503),
        (ErrorCode::Unsupported, 501),
        (ErrorCode::UnsupportedMediaType, 415),
        (ErrorCode::Internal, 500),
    ] {
        assert_eq!(
            code.status().as_u16(),
            status,
            "{code:?} should be {status}"
        );
    }
}

/// Three codes share 503, and the wire `code` is what keeps them distinct.
#[test]
fn the_codes_sharing_a_status_stay_distinguishable() {
    let codes = [
        ErrorCode::PolicyUnavailable,
        ErrorCode::LockContention,
        ErrorCode::Closing,
    ];

    let mut seen = std::collections::BTreeSet::new();
    for code in codes {
        assert_eq!(code.status().as_u16(), 503);
        assert!(
            seen.insert(code.as_str()),
            "`{}` is not unique",
            code.as_str()
        );
        assert!(seen.insert(code.title()), "titles collapse the distinction");
    }
}

#[test]
fn only_the_internal_code_is_sanitized() {
    assert!(ErrorCode::Internal.is_sanitized());

    for code in [
        ErrorCode::NotFound,
        ErrorCode::Forbidden,
        ErrorCode::InvalidAttribute,
        ErrorCode::Conflict,
    ] {
        assert!(!code.is_sanitized(), "{code:?} may show its detail");
    }
}

#[test]
fn the_retriable_codes_are_the_transient_ones() {
    for code in [
        ErrorCode::PolicyUnavailable,
        ErrorCode::LockContention,
        ErrorCode::Closing,
        ErrorCode::Conflict,
    ] {
        assert!(code.is_retriable(), "{code:?} should be retriable");
    }

    for code in [
        ErrorCode::NotFound,
        ErrorCode::Forbidden,
        ErrorCode::InvalidAttribute,
        ErrorCode::Internal,
    ] {
        assert!(!code.is_retriable(), "{code:?} will never succeed on retry");
    }
}

/// The whole point of the rule: a storage message must not reach a client.
#[test]
fn an_internal_errors_detail_is_dropped() {
    let err =
        JsonApiError::internal(&ctx()).detail("postgres://user:hunter2@db.internal/app timed out");

    let object = err.to_object();
    assert_eq!(object.detail.as_deref(), Some(INTERNAL_DETAIL));
    assert!(
        !format!("{object:?}").contains("hunter2"),
        "the credential must not survive anywhere in the object"
    );
}

#[test]
fn a_client_facing_detail_is_kept() {
    let err = JsonApiError::in_request(ErrorCode::NotFound, &ctx()).detail("no todo with that id");

    assert_eq!(
        err.to_object().detail.as_deref(),
        Some("no todo with that id")
    );
}

#[test]
fn an_error_carries_the_correlation_id_as_its_object_id() {
    let object = JsonApiError::in_request(ErrorCode::NotFound, &ctx()).to_object();

    assert_eq!(
        object.id.as_deref(),
        Some("01H9K2QZ8V"),
        "this is what makes `here is my error id` resolvable"
    );
}

#[test]
fn an_error_built_without_a_context_carries_no_id() {
    let object = JsonApiError::new(ErrorCode::NotFound).to_object();
    assert_eq!(object.id, None);
}

#[test]
fn the_object_carries_status_code_and_title_as_strings() {
    let object = JsonApiError::in_request(ErrorCode::Conflict, &ctx()).to_object();

    assert_eq!(object.status, "409");
    assert_eq!(object.code, "conflict");
    assert_eq!(object.title, "Conflict");
}

// --- Pointers ----------------------------------------------------------

#[test]
fn an_attribute_becomes_a_pointer_under_data_attributes() {
    let object = JsonApiError::in_request(ErrorCode::InvalidAttribute, &ctx())
        .attribute("message")
        .to_object();

    assert_eq!(
        object.source.unwrap().pointer.unwrap(),
        "/data/attributes/message"
    );
}

#[test]
fn a_dotted_attribute_nests() {
    let object = JsonApiError::in_request(ErrorCode::InvalidAttribute, &ctx())
        .attribute("address.city")
        .to_object();

    assert_eq!(
        object.source.unwrap().pointer.unwrap(),
        "/data/attributes/address/city"
    );
}

#[test]
fn a_pointer_can_name_any_member() {
    let object = JsonApiError::in_request(ErrorCode::InvalidAttribute, &ctx())
        .pointer("/data/relationships/owner/data")
        .to_object();

    assert_eq!(
        object.source.unwrap().pointer.unwrap(),
        "/data/relationships/owner/data"
    );
}

#[test]
fn a_parameter_is_a_parameter_not_a_pointer() {
    let object = JsonApiError::in_request(ErrorCode::InvalidParameter, &ctx())
        .parameter("sort")
        .to_object();

    let source = object.source.unwrap();
    assert_eq!(source.parameter.unwrap(), "sort");
    assert!(
        source.pointer.is_none(),
        "a query parameter has no document location"
    );
}

#[test]
fn the_invalid_shortcut_sets_both_detail_and_pointer() {
    let object =
        JsonApiError::invalid(&ctx(), "must not be empty", "/data/attributes/message").to_object();

    assert_eq!(object.status, "422");
    assert_eq!(object.detail.as_deref(), Some("must not be empty"));
    assert_eq!(
        object.source.unwrap().pointer.unwrap(),
        "/data/attributes/message"
    );
}

// --- i18n --------------------------------------------------------------

#[test]
fn a_translation_key_rides_under_meta() {
    let object = JsonApiError::in_request(ErrorCode::InvalidAttribute, &ctx())
        .detail("must be 80 characters or fewer")
        .attribute("name")
        .translate(ash_jsonapi::t!("todo.name.too_long", max = 80))
        .to_object();

    let meta = object.meta.expect("meta");
    assert_eq!(meta["i18n"]["key"], "todo.name.too_long");
    assert_eq!(meta["i18n"]["params"]["max"], 80);

    assert_eq!(
        object.detail.as_deref(),
        Some("must be 80 characters or fewer"),
        "the English fallback stays for a client with no catalog"
    );
}

#[test]
fn an_error_without_a_translation_carries_no_meta() {
    let object = JsonApiError::in_request(ErrorCode::NotFound, &ctx()).to_object();

    assert!(
        object.meta.is_none(),
        "an absent member, not an empty object"
    );
}

/// A key names a message this crate chose, so it cannot leak a cause.
#[test]
fn a_sanitized_error_keeps_its_translation_key() {
    let object = JsonApiError::internal(&ctx())
        .detail("connection refused")
        .translate(ash_jsonapi::t!("server.unavailable"))
        .to_object();

    assert_eq!(object.detail.as_deref(), Some(INTERNAL_DETAIL));
    assert_eq!(object.meta.unwrap()["i18n"]["key"], "server.unavailable");
}

#[test]
fn the_translation_reads_back_off_the_error() {
    let err = JsonApiError::in_request(ErrorCode::NotFound, &ctx())
        .translate(ash_jsonapi::t!("todo.not_found"));

    assert_eq!(err.translation().unwrap().key, "todo.not_found");
}

// --- Display -----------------------------------------------------------

#[test]
fn display_shows_the_title_and_any_detail() {
    let err = JsonApiError::in_request(ErrorCode::NotFound, &ctx()).detail("no todo with that id");
    assert_eq!(err.to_string(), "Not found: no todo with that id");

    let bare = JsonApiError::in_request(ErrorCode::NotFound, &ctx());
    assert_eq!(bare.to_string(), "Not found");
}

/// A log line and the response should agree about what was said.
#[test]
fn display_of_a_sanitized_error_shows_only_its_title() {
    let err = JsonApiError::internal(&ctx()).detail("postgres://hunter2@db");

    assert_eq!(err.to_string(), "Internal server error");
    assert!(!err.to_string().contains("hunter2"));
}

#[test]
fn a_locale_travels_from_the_context_onto_the_error() {
    let locale = ash_jsonapi::Locale::negotiate("de-DE").unwrap();
    let ctx = Context::new("c").with_locale(locale);

    let err = JsonApiError::in_request(ErrorCode::NotFound, &ctx);
    assert_eq!(err.locale().unwrap().as_str(), "de-DE");
}
