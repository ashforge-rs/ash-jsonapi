//! `Upload`'s two guards: a size cap, and a media-type allowlist.
use ash_jsonapi::Context;
use ash_jsonapi::extract::Upload;

fn req(ct: Option<&str>, body: Vec<u8>) -> axum::extract::Request {
    let mut b = axum::http::Request::builder().method("POST").uri("/docs");
    if let Some(ct) = ct {
        b = b.header("content-type", ct);
    }
    b.body(axum::body::Body::from(body)).unwrap()
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused() {
    let ctx = Context::new("c");
    let big = vec![0u8; 2048];
    let err = Upload::limited(req(Some("image/png"), big), &ctx, 1024).await;
    assert!(err.is_err(), "2 KiB must not pass a 1 KiB cap");
}

#[tokio::test]
async fn a_body_under_the_limit_passes_with_its_type() {
    let ctx = Context::new("c");
    let up = Upload::limited(req(Some("image/png"), vec![1, 2, 3]), &ctx, 1024)
        .await
        .unwrap();
    assert_eq!(up.len(), 3);
    assert!(!up.is_empty());
    assert_eq!(up.content_type, "image/png");
}

#[tokio::test]
async fn a_missing_content_type_becomes_octet_stream() {
    let ctx = Context::new("c");
    let up = Upload::limited(req(None, vec![1]), &ctx, 1024)
        .await
        .unwrap();
    assert_eq!(up.content_type, "application/octet-stream");
}

#[tokio::test]
async fn an_unaccepted_type_is_refused() {
    let ctx = Context::new("c");
    let res = Upload::accepting(
        req(Some("text/html"), vec![b'<']),
        &ctx,
        1024,
        &["image/png", "image/jpeg"],
    )
    .await;
    // Stored XSS starts exactly here.
    assert!(res.is_err(), "text/html must not pass an image allowlist");
}

#[tokio::test]
async fn parameters_after_the_type_do_not_defeat_the_allowlist() {
    let ctx = Context::new("c");
    let up = Upload::accepting(
        req(Some("image/png; charset=binary"), vec![1]),
        &ctx,
        1024,
        &["image/png"],
    )
    .await;
    assert!(up.is_ok(), "the type alone is what matches");
}

#[tokio::test]
async fn the_allowlist_is_case_insensitive() {
    let ctx = Context::new("c");
    let up = Upload::accepting(req(Some("IMAGE/PNG"), vec![1]), &ctx, 1024, &["image/png"]).await;
    assert!(up.is_ok(), "media types are case-insensitive");
}
