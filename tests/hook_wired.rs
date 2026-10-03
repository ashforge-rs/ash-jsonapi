//! Hooks must fire through the *generated handlers*, not just in isolation.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ash_jsonapi::crud::{Create, Delete, List, Listing, Read, Storage, Update, Validators};
use ash_jsonapi::hook::{Event, Hook, Hooks, Outcome};
use ash_jsonapi::{Context, ErrorCode, JsonApiError, Page, Query};

#[derive(Clone)]
struct Todo {
    id: String,
    message: String,
    done: bool,
}

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);
impl Hook for Log {
    fn before(&self, e: &Event<'_>, _c: &Context) -> Result<(), JsonApiError> {
        self.0.lock().unwrap().push(format!(
            "before:{}:{}",
            e.operation.as_str(),
            e.id.unwrap_or("-")
        ));
        Ok(())
    }
    fn after(&self, e: &Event<'_>, o: &Outcome, _c: &Context) {
        self.0
            .lock()
            .unwrap()
            .push(format!("after:{}:{}", e.operation.as_str(), o.status()));
    }
}

/// Refuses writes, to prove `before` can stop an operation reaching storage.
struct ReadOnly;
impl Hook for ReadOnly {
    fn before(&self, e: &Event<'_>, ctx: &Context) -> Result<(), JsonApiError> {
        if e.operation.is_write() {
            return Err(JsonApiError::in_request(ErrorCode::Forbidden, ctx));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct App {
    rows: Arc<Mutex<BTreeMap<String, Todo>>>,
    reached_storage: Arc<Mutex<bool>>,
    hooks: Hooks,
    validator: Arc<ash_jsonapi::validation::DocumentValidator>,
}

ash_jsonapi::resource! {
    Todo as "todo" at "/todos",
    state: App, schema: TodoSchema, input: NewTodo, id: id,
    attributes: { message: String, done: bool },
}

impl Validators for App {
    fn validator(&self, resource: &str) -> Option<&ash_jsonapi::validation::DocumentValidator> {
        (resource == "todo").then(|| &*self.validator)
    }
    fn hooks(&self) -> Option<&Hooks> {
        Some(&self.hooks)
    }
}

impl Storage<Todo> for App {
    type Error = String;
}
impl Read<Todo> for App {
    async fn get(&self, id: &str) -> Result<Option<Todo>, String> {
        Ok(self.rows.lock().unwrap().get(id).cloned())
    }
}
impl List<Todo> for App {
    async fn list(&self, q: &Query, _c: &Context) -> Result<Listing<Todo>, String> {
        let rows = self.rows.lock().unwrap();
        let Page::Offset { offset, limit } = q.page() else {
            return Err("x".into());
        };
        Ok(Listing::new(
            rows.values().skip(*offset).take(*limit).cloned(),
        ))
    }
}
impl Create<Todo> for App {
    async fn create(&self, input: NewTodo, _c: &Context) -> Result<Todo, String> {
        *self.reached_storage.lock().unwrap() = true;
        let mut rows = self.rows.lock().unwrap();
        let todo = Todo {
            id: "t_new".into(),
            message: input.message,
            done: input.done,
        };
        rows.insert(todo.id.clone(), todo.clone());
        Ok(todo)
    }
}
impl Update<Todo> for App {
    async fn update(&self, id: &str, input: NewTodo, _c: &Context) -> Result<Todo, String> {
        *self.reached_storage.lock().unwrap() = true;
        Ok(Todo {
            id: id.into(),
            message: input.message,
            done: input.done,
        })
    }
}
impl Delete<Todo> for App {
    async fn destroy(&self, _id: &str) -> Result<(), String> {
        *self.reached_storage.lock().unwrap() = true;
        Ok(())
    }
}

ash_jsonapi::api! {
    operations: [ash_jsonapi::CRUD],
    resource: Todo, name: "todo", path: "/todos", state: App,
}

fn app(hooks: Hooks) -> (axum::Router, Arc<Mutex<bool>>) {
    let reached = Arc::new(Mutex::new(false));
    let rows = BTreeMap::from([(
        "t_1".to_string(),
        Todo {
            id: "t_1".into(),
            message: "hi".into(),
            done: false,
        },
    )]);
    let state = App {
        rows: Arc::new(Mutex::new(rows)),
        reached_storage: reached.clone(),
        hooks,
        validator: Arc::new(Todo::validator().unwrap()),
    };
    (
        ash_jsonapi::router!(App, routes()).with_state(state),
        reached,
    )
}

async fn send(router: axum::Router, method: &str, uri: &str, body: Option<&str>) -> u16 {
    use tower::ServiceExt;
    let mut req = axum::http::Request::builder().method(method).uri(uri);
    if body.is_some() {
        req = req.header("content-type", "application/vnd.api+json");
    }
    let req = req
        .body(axum::body::Body::from(body.unwrap_or("").to_string()))
        .unwrap();
    router.oneshot(req).await.unwrap().status().as_u16()
}

#[tokio::test]
async fn every_operation_fires_before_and_after() {
    let log = Log::default();
    let (router, _) = app(Hooks::new().with(log.clone()));

    assert_eq!(send(router.clone(), "GET", "/todos", None).await, 200);
    assert_eq!(send(router.clone(), "GET", "/todos/t_1", None).await, 200);
    let body = r#"{"data":{"type":"todo","attributes":{"message":"x","done":false}}}"#;
    assert_eq!(
        send(router.clone(), "POST", "/todos", Some(body)).await,
        201
    );
    assert_eq!(
        send(router.clone(), "PATCH", "/todos/t_1", Some(body)).await,
        200
    );
    assert_eq!(send(router, "DELETE", "/todos/t_1", None).await, 204);

    let entries = log.0.lock().unwrap().clone();
    assert_eq!(
        entries,
        vec![
            "before:list:-",
            "after:list:200",
            "before:read:t_1",
            "after:read:200",
            "before:create:-",
            "after:create:201",
            "before:update:t_1",
            "after:update:200",
            "before:delete:t_1",
            "after:delete:204",
        ],
        "got: {entries:?}"
    );
}

#[tokio::test]
async fn a_before_hook_stops_the_request_reaching_storage() {
    let (router, reached) = app(Hooks::new().with(ReadOnly));
    let body = r#"{"data":{"type":"todo","attributes":{"message":"x","done":false}}}"#;

    assert_eq!(
        send(router.clone(), "POST", "/todos", Some(body)).await,
        403
    );
    assert_eq!(
        send(router.clone(), "DELETE", "/todos/t_1", None).await,
        403
    );
    assert!(
        !*reached.lock().unwrap(),
        "a refused write must never reach storage"
    );

    // Reads still pass.
    assert_eq!(send(router, "GET", "/todos/t_1", None).await, 200);
}

#[tokio::test]
async fn after_sees_the_real_failure_status() {
    let log = Log::default();
    let (router, _) = app(Hooks::new().with(log.clone()));
    assert_eq!(send(router, "GET", "/todos/nope", None).await, 404);
    let entries = log.0.lock().unwrap().clone();
    assert!(
        entries.contains(&"after:read:404".to_string()),
        "got: {entries:?}"
    );
}
