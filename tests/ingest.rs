//! JSONL ingest: one result line per input line, and partial success.
use std::sync::{Arc, Mutex};

use ash_jsonapi::crud::{Create, Storage};
use ash_jsonapi::{Context, JsonApiError};

#[derive(Clone)]
struct Todo {
    id: String,
    message: String,
}

#[derive(Clone)]
struct App {
    rows: Arc<Mutex<Vec<Todo>>>,
    validator: Arc<ash_jsonapi::validation::DocumentValidator>,
    /// Storage refuses any message containing this, to exercise a failure
    /// that is storage's rather than validation's.
    poison: Option<String>,
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

impl Create<Todo> for App {
    async fn create(&self, input: NewTodo, _ctx: &Context) -> Result<Todo, String> {
        if let Some(poison) = &self.poison
            && input.message.contains(poison.as_str())
        {
            return Err("storage refused the row".to_string());
        }

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
    operations: [ash_jsonapi::CREATE, ash_jsonapi::INGEST],
    resource: Todo, name: "todo", path: "/todos", state: App,
}

fn app(poison: Option<&str>) -> axum::Router {
    ash_jsonapi::router!(App, routes()).with_state(App {
        rows: Arc::new(Mutex::new(Vec::new())),
        validator: Arc::new(Todo::validator().expect("the schema compiles")),
        poison: poison.map(str::to_string),
    })
}

/// One JSONL line for a todo with this message.
fn line(message: &str) -> String {
    serde_json::json!({ "data": { "type": "todo", "attributes": { "message": message } } })
        .to_string()
}

struct Res {
    status: u16,
    content_type: String,
    body: String,
}

async fn post(app: &axum::Router, content_type: &str, body: String) -> Res {
    use tower::ServiceExt;
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/todos")
        .header("content-type", content_type)
        .body(axum::body::Body::from(body))
        .unwrap();

    let res = app.clone().oneshot(request).await.unwrap();
    let status = res.status().as_u16();
    let content_type = res
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();

    Res {
        status,
        content_type,
        body: String::from_utf8(body.to_vec()).unwrap(),
    }
}

/// Every line, parsed as JSON.
fn lines(body: &str) -> Vec<serde_json::Value> {
    body.lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|_| panic!("not JSON: {l}")))
        .collect()
}

#[tokio::test]
async fn every_line_becomes_one_result_line() {
    let body = [line("buy milk"), line("write docs"), line("ship it")].join("\n");
    let res = post(&app(None), "application/jsonl", body).await;

    assert_eq!(res.status, 200);
    assert!(
        res.content_type.starts_with("application/jsonl"),
        "{}",
        res.content_type
    );

    let out = lines(&res.body);
    assert_eq!(out.len(), 3, "one result per input line: {}", res.body);

    for (n, value) in out.iter().enumerate() {
        assert_eq!(value["data"]["type"], "todo");
        assert_eq!(value["data"]["id"], format!("t_{}", n + 1));
    }
}

#[tokio::test]
async fn a_bad_line_does_not_stop_the_batch() {
    // The middle line is missing `message`, which the schema requires.
    let bad = serde_json::json!({ "data": { "type": "todo", "attributes": {} } }).to_string();
    let body = [line("first"), bad, line("third")].join("\n");

    let res = post(&app(None), "application/jsonl", body).await;
    assert_eq!(
        res.status, 200,
        "the status is committed with the first byte"
    );

    let out = lines(&res.body);
    assert_eq!(
        out.len(),
        3,
        "the run continued past the failure: {}",
        res.body
    );

    assert_eq!(out[0]["data"]["id"], "t_1");
    assert!(out[1].get("errors").is_some(), "line 2 failed: {}", out[1]);
    // The third line still landed, and kept the next id.
    assert_eq!(out[2]["data"]["id"], "t_2");
}

#[tokio::test]
async fn an_error_line_names_the_line_it_came_from() {
    let bad = serde_json::json!({ "data": { "type": "todo", "attributes": {} } }).to_string();
    let body = [line("first"), line("second"), bad].join("\n");

    let res = post(&app(None), "application/jsonl", body).await;
    let out = lines(&res.body);

    let errors = out[2]["errors"].as_array().expect("an error line");
    assert_eq!(
        errors[0]["meta"]["line"], 3,
        "a client must be able to map the failure back onto its input: {}",
        out[2]
    );
}

#[tokio::test]
async fn a_validation_failure_keeps_its_status_and_pointer() {
    let bad = serde_json::json!({ "data": { "type": "todo", "attributes": {} } }).to_string();
    let res = post(&app(None), "application/jsonl", bad).await;

    let out = lines(&res.body);
    let error = &out[0]["errors"][0];

    assert_eq!(error["status"], "422", "not collapsed into a 500: {error}");
    assert_eq!(error["code"], "invalid_attribute");
    assert!(
        error["source"]["pointer"].as_str().is_some(),
        "the schema reports the instance location: {error}"
    );
}

#[tokio::test]
async fn a_storage_failure_is_a_sanitized_internal_error() {
    let body = [line("fine"), line("poisoned row")].join("\n");
    let res = post(&app(Some("poisoned")), "application/jsonl", body).await;

    let out = lines(&res.body);
    assert_eq!(out[0]["data"]["id"], "t_1");

    let error = &out[1]["errors"][0];
    assert_eq!(error["status"], "500");
    assert_eq!(
        error["detail"], "An internal error occurred.",
        "the storage message must not reach the client: {error}"
    );
    assert_eq!(error["meta"]["line"], 2);
}

#[tokio::test]
async fn a_trailing_newline_is_not_an_empty_row() {
    let body = format!("{}\n{}\n", line("one"), line("two"));
    let res = post(&app(None), "application/jsonl", body).await;

    let out = lines(&res.body);
    assert_eq!(
        out.len(),
        2,
        "the trailing newline added no row: {}",
        res.body
    );
}

#[tokio::test]
async fn blank_lines_between_rows_are_skipped() {
    let body = format!("{}\n\n\n{}", line("one"), line("two"));
    let res = post(&app(None), "application/jsonl", body).await;

    let out = lines(&res.body);
    assert_eq!(out.len(), 2, "{}", res.body);
    // And the numbering still counts real rows only.
    assert_eq!(out[1]["data"]["id"], "t_2");
}

#[tokio::test]
async fn a_final_line_without_a_newline_still_counts() {
    let body = format!("{}\n{}", line("one"), line("two"));
    let res = post(&app(None), "application/jsonl", body).await;

    assert_eq!(lines(&res.body).len(), 2, "{}", res.body);
}

#[tokio::test]
async fn ndjson_is_accepted_as_the_same_thing() {
    let res = post(&app(None), "application/x-ndjson", line("one")).await;
    assert_eq!(res.status, 200);
    assert_eq!(lines(&res.body).len(), 1);
}

#[tokio::test]
async fn a_jsonapi_body_is_still_a_single_create() {
    let res = post(&app(None), "application/vnd.api+json", line("just one")).await;

    assert_eq!(res.status, 201, "routing INGEST must not change CREATE");
    assert!(
        res.content_type.starts_with("application/vnd.api+json"),
        "{}",
        res.content_type
    );

    let document: serde_json::Value = serde_json::from_str(&res.body).unwrap();
    assert_eq!(document["data"]["id"], "t_1");
}

#[tokio::test]
async fn a_charset_parameter_does_not_defeat_negotiation() {
    let res = post(&app(None), "application/jsonl; charset=utf-8", line("one")).await;
    assert_eq!(res.status, 200);
    assert!(res.content_type.starts_with("application/jsonl"));
}

#[tokio::test]
async fn an_empty_body_is_an_empty_batch() {
    let res = post(&app(None), "application/jsonl", String::new()).await;

    assert_eq!(res.status, 200);
    assert!(res.body.is_empty(), "no rows, no lines: {:?}", res.body);
}

/// The decoder's cap is what keeps an unbounded body from being buffered.
#[tokio::test]
async fn a_line_over_the_cap_is_refused_rather_than_buffered() {
    use ash_jsonapi::extract::JsonlLines;
    use futures_util::StreamExt;

    let ctx = Context::new("c");
    // One line, far over the limit, with no newline to terminate it.
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/todos")
        .body(axum::body::Body::from(vec![b'x'; 4096]))
        .unwrap();

    let mut decoded = JsonlLines::limited(request, &ctx, 1024);
    let (number, result) = decoded.next().await.expect("one item");

    assert_eq!(number, 1);
    let error: JsonApiError = result.expect_err("over the cap");
    assert_eq!(error.status().as_u16(), 422);
}
