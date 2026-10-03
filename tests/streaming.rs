//! JSONL streaming: negotiation, framing, and failing partway.
use std::sync::{Arc, Mutex};

use ash_jsonapi::crud::{List, Listing, Storage, Stream, Validators};
use ash_jsonapi::{Context, Query};

#[derive(Clone)]
struct Todo {
    id: String,
    message: String,
}

#[derive(Clone)]
struct App {
    rows: Arc<Mutex<Vec<Todo>>>,
    fail_at: Option<usize>,
}
impl Validators for App {}

ash_jsonapi::resource! {
    Todo as "todo" at "/todos",
    state: App, schema: TodoSchema, input: NewTodo, id: id,
    attributes: { message: String },
}

impl Storage<Todo> for App {
    type Error = String;
}

impl List<Todo> for App {
    async fn list(&self, _q: &Query, _c: &Context) -> Result<Listing<Todo>, String> {
        Ok(Listing::new(self.rows.lock().unwrap().clone()))
    }
}

impl Stream<Todo> for App {
    fn stream(
        self,
        _q: Query,
        _c: Context,
    ) -> impl futures_core::Stream<Item = Result<Todo, String>> + Send + Unpin + 'static {
        let rows = self.rows.lock().unwrap().clone();
        let fail_at = self.fail_at;
        let items: Vec<Result<Todo, String>> = rows
            .into_iter()
            .enumerate()
            .map(|(n, row)| match fail_at {
                Some(at) if n == at => Err("storage exploded".to_string()),
                _ => Ok(row),
            })
            .collect();
        futures_util::stream::iter(items)
    }
}

ash_jsonapi::api! {
    operations: [ash_jsonapi::LIST, ash_jsonapi::STREAM],
    resource: Todo, name: "todo", path: "/todos", state: App,
}

fn app(fail_at: Option<usize>) -> axum::Router {
    let rows = (1..=3)
        .map(|n| Todo {
            id: format!("t_{n}"),
            message: format!("row {n}"),
        })
        .collect();
    ash_jsonapi::router!(App, routes()).with_state(App {
        rows: Arc::new(Mutex::new(rows)),
        fail_at,
    })
}

async fn get(app: &axum::Router, accept: Option<&str>) -> (u16, String, String) {
    use tower::ServiceExt;
    let mut req = axum::http::Request::builder().method("GET").uri("/todos");
    if let Some(a) = accept {
        req = req.header("accept", a);
    }
    let res = app
        .clone()
        .oneshot(req.body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status().as_u16();
    let ct = res
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, ct, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn jsonl_yields_one_object_per_line() {
    let (status, ct, body) = get(&app(None), Some("application/jsonl")).await;
    assert_eq!(status, 200);
    assert!(ct.starts_with("application/jsonl"), "{ct}");

    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 3, "one line per row: {body:?}");
    for (n, line) in lines.iter().enumerate() {
        let v: serde_json::Value = serde_json::from_str(line).expect("each line is valid JSON");
        assert_eq!(v["type"], "todo");
        assert_eq!(v["id"], format!("t_{}", n + 1));
    }
}

#[tokio::test]
async fn ndjson_is_accepted_too() {
    let (_, ct, _) = get(&app(None), Some("application/x-ndjson")).await;
    assert!(ct.starts_with("application/jsonl"), "{ct}");
}

#[tokio::test]
async fn the_document_is_still_the_default() {
    for accept in [None, Some("*/*"), Some("application/vnd.api+json")] {
        let (status, ct, body) = get(&app(None), accept).await;
        assert_eq!(status, 200);
        assert!(
            ct.starts_with("application/vnd.api+json"),
            "accept={accept:?} ct={ct}"
        );
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["data"].is_array(), "a document, not lines");
    }
}

#[tokio::test]
async fn a_response_varies_on_accept() {
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/todos")
        .header("accept", "application/jsonl")
        .body(axum::body::Body::empty())
        .unwrap();
    let res = app(None).oneshot(req).await.unwrap();
    assert_eq!(res.headers()["vary"], "Accept", "caches must key on Accept");
}

#[tokio::test]
async fn failing_partway_ends_with_an_error_line_not_a_truncation() {
    // Row 1 succeeds, row 2 fails. The status is already 200 by then.
    let (status, _, body) = get(&app(Some(1)), Some("application/jsonl")).await;
    assert_eq!(status, 200, "the status was committed with the first byte");

    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 2, "one row, then the error: {body:?}");

    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["id"], "t_1");

    // The last line is what tells a client the export is not complete.
    let last: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert!(
        last.get("errors").is_some(),
        "must be an error line: {}",
        lines[1]
    );
    assert_eq!(last["errors"][0]["status"], "500");
}

#[tokio::test]
async fn an_immediate_failure_is_still_a_valid_line() {
    let (status, _, body) = get(&app(Some(0)), Some("application/jsonl")).await;
    assert_eq!(status, 200);
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 1);
    let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert!(v.get("errors").is_some(), "{body}");
}
