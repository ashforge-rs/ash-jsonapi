//! A collection that routes `INGEST` without `CREATE`: batches, and nothing else.
//!
//! The other half of the `POST` dispatcher. With `CREATE` alongside it, a
//! non-JSONL body is an ordinary single create; with `INGEST` alone there is
//! no single create to fall back to, so the same body is a `415`.
use std::sync::{Arc, Mutex};

use ash_jsonapi::Context;
use ash_jsonapi::crud::{Create, Storage};

#[derive(Clone)]
struct Todo {
    id: String,
    message: String,
}

#[derive(Clone)]
struct App {
    rows: Arc<Mutex<Vec<Todo>>>,
    validator: Arc<ash_jsonapi::validation::DocumentValidator>,
}

ash_jsonapi::resource! {
    Todo as "todo" at "/todos",
    state: App, schema: TodoSchema, input: NewTodo, id: id,
    attributes: { message: String },
}

ash_jsonapi::registry! {
    App { validator: Todo }
}

impl Storage<Todo> for App {
    type Error = String;
}

// `Ingest` comes free from the blanket impl over `Create`, so a batch-only
// collection still writes just this.
impl Create<Todo> for App {
    async fn create(&self, input: NewTodo, _ctx: &Context) -> Result<Todo, String> {
        let mut rows = self.rows.lock().unwrap();
        let todo = Todo {
            id: format!("t_{}", rows.len() + 1),
            message: input.message,
        };
        rows.push(todo.clone());
        Ok(todo)
    }
}

ash_jsonapi::api! {
    operations: [ash_jsonapi::INGEST],
    resource: Todo, name: "todo", path: "/todos", state: App,
}

fn app() -> axum::Router {
    ash_jsonapi::router!(App, routes()).with_state(App {
        rows: Arc::new(Mutex::new(Vec::new())),
        validator: Arc::new(Todo::validator().expect("the schema compiles")),
    })
}

async fn post(content_type: &str, body: String) -> (u16, String) {
    use tower::ServiceExt;
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/todos")
        .header("content-type", content_type)
        .body(axum::body::Body::from(body))
        .unwrap();

    let res = app().oneshot(request).await.unwrap();
    let status = res.status().as_u16();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn line(message: &str) -> String {
    serde_json::json!({ "data": { "type": "todo", "attributes": { "message": message } } })
        .to_string()
}

#[tokio::test]
async fn a_batch_is_accepted() {
    let (status, body) = post("application/jsonl", [line("one"), line("two")].join("\n")).await;

    assert_eq!(status, 200);
    assert_eq!(body.lines().count(), 2, "{body}");
}

#[tokio::test]
async fn a_document_body_is_an_unsupported_media_type() {
    let (status, body) = post("application/vnd.api+json", line("one")).await;

    assert_eq!(
        status, 415,
        "there is no single create to fall back to: {body}"
    );

    let document: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(document["errors"][0]["code"], "unsupported_media_type");
}

#[tokio::test]
async fn the_collection_has_no_get() {
    use tower::ServiceExt;
    let request = axum::http::Request::builder()
        .method("GET")
        .uri("/todos")
        .body(axum::body::Body::empty())
        .unwrap();

    let res = app().oneshot(request).await.unwrap();
    assert_eq!(
        res.status().as_u16(),
        405,
        "INGEST routes a POST and nothing else"
    );
}
