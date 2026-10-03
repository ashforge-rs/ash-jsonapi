//! `Storage::sortable` must refuse an unknown key before storage runs.
use std::sync::{Arc, Mutex};

use ash_jsonapi::crud::{List, Listing, Storage, Validators};
use ash_jsonapi::{Context, Query};

#[derive(Clone)]
struct Todo {
    id: String,
    message: String,
}

#[derive(Clone)]
struct App {
    reached: Arc<Mutex<bool>>,
}

ash_jsonapi::resource! {
    Todo as "todo" at "/todos",
    state: App, schema: TodoSchema, input: NewTodo, id: id,
    attributes: { message: String },
}
impl Validators for App {}

impl Storage<Todo> for App {
    type Error = String;
    fn sortable(&self) -> Option<&[&str]> {
        Some(&["id", "message"])
    }
}
impl List<Todo> for App {
    async fn list(&self, _q: &Query, _c: &Context) -> Result<Listing<Todo>, String> {
        *self.reached.lock().unwrap() = true;
        Ok(Listing::new(Vec::new()))
    }
}

ash_jsonapi::api! {
    operations: [ash_jsonapi::LIST],
    resource: Todo, name: "todo", path: "/todos", state: App,
}

async fn get(uri: &str) -> (u16, String, Arc<Mutex<bool>>) {
    use tower::ServiceExt;
    let reached = Arc::new(Mutex::new(false));
    let router = ash_jsonapi::router!(App, routes()).with_state(App {
        reached: reached.clone(),
    });
    let req = axum::http::Request::builder()
        .method("GET")
        .uri(uri)
        .body(axum::body::Body::empty())
        .unwrap();
    let res = router.oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap(), reached)
}

#[tokio::test]
async fn an_unknown_sort_field_is_a_400_before_storage() {
    let (status, body, reached) = get("/todos?sort=secret").await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("cannot sort on `secret`"), "{body}");
    assert!(body.contains("\"parameter\":\"sort\""), "{body}");
    assert!(!*reached.lock().unwrap(), "storage must not be reached");
}

#[tokio::test]
async fn the_message_names_what_is_allowed() {
    let (_, body, _) = get("/todos?sort=secret").await;
    assert!(
        body.contains("`id`, `message`"),
        "should list the options: {body}"
    );
}

#[tokio::test]
async fn a_declared_field_passes_through() {
    let (status, _, reached) = get("/todos?sort=-message").await;
    assert_eq!(status, 200);
    assert!(*reached.lock().unwrap(), "a valid sort must reach storage");
}

#[tokio::test]
async fn no_sort_at_all_is_fine() {
    let (status, _, reached) = get("/todos").await;
    assert_eq!(status, 200);
    assert!(*reached.lock().unwrap());
}
