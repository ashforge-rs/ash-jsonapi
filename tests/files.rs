//! Upload → metadata → download, through the generated routes.
use std::sync::{Arc, Mutex};

use ash_jsonapi::crud::{Content, Create, Read, Resource, Storage, Validators};
use ash_jsonapi::document::ResourceObject;
use ash_jsonapi::extract::Upload;
use ash_jsonapi::response::Download;
use ash_jsonapi::{Context, JsonApiError};

#[derive(Clone)]
struct Doc {
    id: String,
    bytes: Vec<u8>,
    content_type: String,
}

#[derive(Clone)]
struct App {
    rows: Arc<Mutex<Vec<Doc>>>,
}
impl Validators for App {}

impl Resource<App> for Doc {
    type Create = Upload;
    type Update = Upload;
    const NAME: &'static str = "doc";
    const PATH: &'static str = "/docs";
    fn to_resource(&self, _s: &App, _c: &Context) -> Result<ResourceObject, JsonApiError> {
        Ok(ResourceObject::new("doc", &self.id)
            .attr("content_type", self.content_type.clone())
            .attr("size", self.bytes.len() as u64))
    }
}

impl Storage<Doc> for App {
    type Error = String;
}

impl Create<Doc> for App {
    async fn create(&self, input: Upload, _c: &Context) -> Result<Doc, String> {
        let mut rows = self.rows.lock().unwrap();
        let doc = Doc {
            id: format!("d_{}", rows.len() + 1),
            bytes: input.bytes,
            content_type: input.content_type,
        };
        rows.push(doc.clone());
        Ok(doc)
    }
}

impl Read<Doc> for App {
    async fn get(&self, id: &str) -> Result<Option<Doc>, String> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|d| d.id == id)
            .cloned())
    }
}

impl Content<Doc> for App {
    async fn content(&self, id: &str) -> Result<Option<Download>, String> {
        let Some(doc) = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|d| d.id == id)
            .cloned()
        else {
            return Ok(None);
        };
        Ok(Some(
            Download::new(doc.bytes, doc.content_type).filename(format!("{id}.bin")),
        ))
    }
}

ash_jsonapi::api! {
    operations: [ash_jsonapi::CREATE, ash_jsonapi::READ, ash_jsonapi::CONTENT],
    resource: Doc, name: "doc", path: "/docs", state: App,
}

fn router() -> axum::Router {
    ash_jsonapi::router!(App, routes()).with_state(App {
        rows: Arc::new(Mutex::new(Vec::new())),
    })
}

struct Res {
    status: u16,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

async fn send(r: &axum::Router, method: &str, uri: &str, ct: Option<&str>, body: Vec<u8>) -> Res {
    use tower::ServiceExt;
    let mut req = axum::http::Request::builder().method(method).uri(uri);
    if let Some(ct) = ct {
        req = req.header("content-type", ct);
    }
    let req = req.body(axum::body::Body::from(body)).unwrap();
    let res = r.clone().oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let headers = res.headers().clone();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Res {
        status,
        headers,
        body,
    }
}

const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[tokio::test]
async fn upload_then_read_metadata_then_download() {
    let r = router();

    // Upload: raw bytes, not a JSON:API document.
    let up = send(&r, "POST", "/docs", Some("image/png"), PNG.to_vec()).await;
    assert_eq!(up.status, 201, "{}", String::from_utf8_lossy(&up.body));
    assert_eq!(up.headers["content-type"], "application/vnd.api+json");

    // The row is an ordinary JSON:API document describing the file.
    let meta = send(&r, "GET", "/docs/d_1", None, vec![]).await;
    assert_eq!(meta.status, 200);
    let json: serde_json::Value = serde_json::from_slice(&meta.body).unwrap();
    assert_eq!(json["data"]["attributes"]["content_type"], "image/png");
    assert_eq!(json["data"]["attributes"]["size"], 8);

    // The bytes live beside it, served as themselves.
    let dl = send(&r, "GET", "/docs/d_1/content", None, vec![]).await;
    assert_eq!(dl.status, 200);
    assert_eq!(dl.headers["content-type"], "image/png");
    assert_eq!(dl.body, PNG, "the exact bytes come back");
    assert_eq!(dl.headers["x-content-type-options"], "nosniff");
    assert!(
        dl.headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("attachment")
    );
}

#[tokio::test]
async fn a_missing_file_is_a_jsonapi_404() {
    let r = router();
    let dl = send(&r, "GET", "/docs/nope/content", None, vec![]).await;
    assert_eq!(dl.status, 404);
    // The error is still a JSON:API document, even on the bytes route.
    let json: serde_json::Value = serde_json::from_slice(&dl.body).unwrap();
    assert_eq!(json["errors"][0]["status"], "404");
}

#[tokio::test]
async fn content_is_not_routed_unless_asked_for() {
    // This resource routes CREATE + READ + CONTENT; a resource without
    // CONTENT must not answer on /{id}/content at all.
    let r = router();
    let other = send(&r, "POST", "/docs/d_1/content", None, vec![]).await;
    assert_eq!(other.status, 405, "only GET is routed there");
}
