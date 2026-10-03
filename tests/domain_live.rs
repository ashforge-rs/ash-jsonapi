//! The `ash-domain` execution path, over HTTP, against a real domain.
//!
//! [`domain.rs`](domain.rs) covers the pure conversions. This covers the half
//! that needs a live domain: `DomainState`, the traits `domain_resource!`
//! writes, and the round trip a request actually makes — document in, through
//! the domain's own pipeline (policies included), document out.
//!
//! The data layer is ash-domain's in-memory one, so there is no database and
//! no fixture beyond a `Domain` and a store.
use std::sync::Arc;

use ash_jsonapi::__ash_domain as ash_domain;
use ash_jsonapi::Context;
use ash_jsonapi::domain::{DomainState, Mounted, MountedSchemas};
use ash_jsonapi::validation::DocumentValidator;

use ash_domain::datalayer::memory::InMemoryDataLayer;
use ash_domain::{Domain, PolicySet, Resource};

#[derive(Resource, Default, Debug, Clone)]
#[resource(name = "note")]
struct Note {
    #[attribute(primary_key)]
    id: Option<String>,
    title: String,
    #[attribute(default = false)]
    done: bool,
}

#[derive(Clone)]
struct App {
    domain: Arc<Domain>,
    store: Arc<InMemoryDataLayer>,
    schemas: Arc<MountedSchemas>,
    validator: Arc<DocumentValidator>,
}

impl DomainState for App {
    type Backend = Arc<InMemoryDataLayer>;

    fn domain(&self) -> &Domain {
        &self.domain
    }

    /// A fresh domain context per request: it holds the actor and tenant the
    /// policies evaluate against, which belong to one request.
    fn domain_context(&self, ctx: &Context) -> ash_domain::Context<Self::Backend> {
        let mut dctx = ash_domain::Context::new(self.store.clone());
        if let Some(tenant) = ctx.tenant() {
            dctx.set_tenant(tenant);
        }
        dctx
    }

    fn mounted(&self, resource: &str) -> Option<Mounted<'_>> {
        self.schemas.get(resource)
    }
}

ash_jsonapi::registry! {
    App { validator: Note }
}

ash_jsonapi::domain_api! {
    Note as "note" at "/notes",
    state: App,
    // `CRUD` already covers LIST, CREATE, READ, UPDATE and DELETE; naming
    // LIST beside it would generate the handler twice.
    operations: [ash_jsonapi::CRUD],
}

/// A router over a domain with the given policies.
///
/// `permissive` is the ordinary case; an empty `PolicySet` is default-deny,
/// which is how the denial path is exercised.
fn app_with(policies: PolicySet) -> axum::Router {
    let domain = Domain::builder()
        .register::<Note>()
        .policies(policies)
        .build();

    let schemas =
        MountedSchemas::new(&domain, [("note", "/notes")]).expect("the domain registers `note`");

    let schema = domain
        .schema()
        .resource("note")
        .expect("registered")
        .clone();

    let state = App {
        domain: Arc::new(domain),
        store: Arc::new(InMemoryDataLayer::new()),
        schemas: Arc::new(schemas),
        validator: Arc::new(DocumentValidator::for_create(&schema).expect("the schema compiles")),
    };

    ash_jsonapi::router!(App, routes()).with_state(state)
}

fn app() -> axum::Router {
    app_with(PolicySet::permissive())
}

struct Res {
    status: u16,
    body: serde_json::Value,
}

async fn send(app: &axum::Router, method: &str, uri: &str, body: Option<serde_json::Value>) -> Res {
    use tower::ServiceExt;

    let mut request = axum::http::Request::builder().method(method).uri(uri);
    let body = match body {
        Some(document) => {
            request = request.header("content-type", ash_jsonapi::MEDIA_TYPE);
            axum::body::Body::from(document.to_string())
        }
        None => axum::body::Body::empty(),
    };

    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();

    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    Res {
        status,
        // A 204 has no body, and a JSONL/empty body is not JSON.
        body: serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    }
}

fn note(title: &str) -> serde_json::Value {
    serde_json::json!({ "data": { "type": "note", "attributes": { "title": title } } })
}

/// Create one note and hand back its id.
async fn seed(app: &axum::Router, title: &str) -> String {
    let created = send(app, "POST", "/notes", Some(note(title))).await;
    assert_eq!(created.status, 201, "{:?}", created.body);
    created.body["data"]["id"]
        .as_str()
        .expect("the domain stamps a primary key")
        .to_string()
}

// --- The round trip ----------------------------------------------------

#[tokio::test]
async fn a_create_runs_through_the_domain_and_comes_back_as_a_document() {
    let app = app();
    let created = send(&app, "POST", "/notes", Some(note("buy milk"))).await;

    assert_eq!(created.status, 201, "{:?}", created.body);
    assert_eq!(created.body["data"]["type"], "note");
    assert_eq!(created.body["data"]["attributes"]["title"], "buy milk");
    assert!(
        created.body["data"]["id"].is_string(),
        "the domain assigns the id: {:?}",
        created.body
    );
}

/// The declared default is the domain's, not this crate's — the point of
/// running through the domain rather than around it.
#[tokio::test]
async fn a_declared_default_is_applied_by_the_domain() {
    let app = app();
    let created = send(&app, "POST", "/notes", Some(note("buy milk"))).await;

    assert_eq!(
        created.body["data"]["attributes"]["done"], false,
        "`#[attribute(default = false)]` filled this in: {:?}",
        created.body
    );
}

#[tokio::test]
async fn a_created_note_reads_back_by_id() {
    let app = app();
    let id = seed(&app, "buy milk").await;

    let fetched = send(&app, "GET", &format!("/notes/{id}"), None).await;

    assert_eq!(fetched.status, 200, "{:?}", fetched.body);
    assert_eq!(fetched.body["data"]["id"], id);
    assert_eq!(fetched.body["data"]["attributes"]["title"], "buy milk");
}

#[tokio::test]
async fn a_missing_id_is_a_404() {
    let app = app();
    let fetched = send(&app, "GET", "/notes/no-such-id", None).await;

    assert_eq!(fetched.status, 404, "{:?}", fetched.body);
    assert_eq!(fetched.body["errors"][0]["code"], "not_found");
}

#[tokio::test]
async fn an_update_persists_and_the_path_id_wins() {
    let app = app();
    let id = seed(&app, "buy milk").await;

    let updated = send(
        &app,
        "PATCH",
        &format!("/notes/{id}"),
        Some(serde_json::json!({
            "data": {
                "type": "note",
                // A body id that disagrees with the path must not decide
                // which row is written.
                "id": "some-other-id",
                "attributes": { "title": "buy oat milk" }
            }
        })),
    )
    .await;

    assert_eq!(updated.status, 200, "{:?}", updated.body);
    assert_eq!(
        updated.body["data"]["id"], id,
        "the path's id is authoritative"
    );
    assert_eq!(updated.body["data"]["attributes"]["title"], "buy oat milk");

    // And it really landed in the store.
    let fetched = send(&app, "GET", &format!("/notes/{id}"), None).await;
    assert_eq!(fetched.body["data"]["attributes"]["title"], "buy oat milk");
}

#[tokio::test]
async fn a_delete_removes_the_row() {
    let app = app();
    let id = seed(&app, "buy milk").await;

    let deleted = send(&app, "DELETE", &format!("/notes/{id}"), None).await;
    assert_eq!(deleted.status, 204);

    let fetched = send(&app, "GET", &format!("/notes/{id}"), None).await;
    assert_eq!(fetched.status, 404, "the row is gone");
}

// --- Listing -----------------------------------------------------------

#[tokio::test]
async fn a_listing_returns_every_row() {
    let app = app();
    for title in ["one", "two", "three"] {
        seed(&app, title).await;
    }

    let listed = send(&app, "GET", "/notes", None).await;

    assert_eq!(listed.status, 200, "{:?}", listed.body);
    let rows = listed.body["data"].as_array().expect("a collection");
    assert_eq!(rows.len(), 3, "{:?}", listed.body);
}

/// `probe_query` reads one row past the page, so `next` is a fact rather than
/// a guess — a full last page must not link into an empty one.
#[tokio::test]
async fn next_is_emitted_exactly_when_there_is_another_page() {
    let app = app();
    for title in ["one", "two", "three", "four"] {
        seed(&app, title).await;
    }

    // Two of four: there is more.
    let first = send(&app, "GET", "/notes?page[limit]=2", None).await;
    assert_eq!(first.body["data"].as_array().unwrap().len(), 2);
    assert!(
        first.body["links"]["next"].is_string(),
        "two of four rows: {:?}",
        first.body["links"]
    );

    // A page that exactly consumes the collection has no next.
    let all = send(&app, "GET", "/notes?page[limit]=4", None).await;
    assert_eq!(all.body["data"].as_array().unwrap().len(), 4);
    assert!(
        all.body["links"]["next"].is_null(),
        "a full page that ends the collection must not link onward: {:?}",
        all.body["links"]
    );
}

#[tokio::test]
async fn an_offset_page_skips_the_rows_before_it() {
    let app = app();
    for title in ["one", "two", "three"] {
        seed(&app, title).await;
    }

    let page = send(&app, "GET", "/notes?page[offset]=2&page[limit]=2", None).await;

    assert_eq!(page.status, 200, "{:?}", page.body);
    assert_eq!(
        page.body["data"].as_array().unwrap().len(),
        1,
        "one row left after skipping two: {:?}",
        page.body
    );
}

/// The domain resumes from a typed cursor, not an opaque string, so a
/// `page[cursor]` this crate handed out cannot be translated back.
#[tokio::test]
async fn cursor_paging_is_refused_rather_than_mistranslated() {
    let app = app();
    let listed = send(&app, "GET", "/notes?page[cursor]=abc&page[size]=2", None).await;

    assert_eq!(listed.status, 501, "{:?}", listed.body);
    assert_eq!(listed.body["errors"][0]["code"], "unsupported");
}

// --- Validation, before the domain is reached --------------------------

#[tokio::test]
async fn a_missing_required_attribute_is_a_422_with_a_pointer() {
    let app = app();
    let created = send(
        &app,
        "POST",
        "/notes",
        Some(serde_json::json!({ "data": { "type": "note", "attributes": {} } })),
    )
    .await;

    assert_eq!(created.status, 422, "{:?}", created.body);
    assert!(
        created.body["errors"][0]["source"]["pointer"]
            .as_str()
            .is_some(),
        "the validator reports the member at fault: {:?}",
        created.body
    );
}

#[tokio::test]
async fn a_wrongly_typed_attribute_is_refused() {
    let app = app();
    let created = send(
        &app,
        "POST",
        "/notes",
        Some(serde_json::json!({
            "data": { "type": "note", "attributes": { "title": 42 } }
        })),
    )
    .await;

    assert_eq!(created.status, 422, "{:?}", created.body);
}

/// The primary key is carried in the envelope, so sending it as an attribute
/// is not a way to set it.
///
/// Hiding it from the create schema is not what enforces this: `hide` removes
/// the property's *description*, and nothing sets
/// `additionalProperties: false`, so the member is ignored rather than
/// refused. The enforcement is on the reading side — `read_document` skips the
/// primary key, mirroring `record_from`, which drops it on the way out.
#[tokio::test]
async fn the_domain_assigns_the_id_not_the_client() {
    let app = app();
    let created = send(
        &app,
        "POST",
        "/notes",
        Some(serde_json::json!({
            "data": {
                "type": "note",
                "attributes": { "title": "buy milk", "id": "chosen-by-client" }
            }
        })),
    )
    .await;

    // Either the schema refuses the member outright, or it is ignored and the
    // domain stamps its own id. Both are correct; silently honouring it is not.
    if created.status == 201 {
        assert_ne!(
            created.body["data"]["id"], "chosen-by-client",
            "a client must not choose the primary key: {:?}",
            created.body
        );
    } else {
        assert_eq!(created.status, 422, "{:?}", created.body);
    }
}

// --- Policies ----------------------------------------------------------

/// The reason actions run *through* `Bound` rather than around it: a policy
/// denial has to reach the client as `403`, not as an opaque `500`.
#[tokio::test]
async fn a_policy_denial_is_a_403() {
    // An empty `PolicySet` is default-deny: no policy admits the write.
    let app = app_with(PolicySet::new());

    let created = send(&app, "POST", "/notes", Some(note("buy milk"))).await;

    assert_eq!(created.status, 403, "{:?}", created.body);
    assert_eq!(created.body["errors"][0]["code"], "forbidden");
}

#[tokio::test]
async fn a_denied_read_is_also_a_403() {
    let app = app_with(PolicySet::new());

    let fetched = send(&app, "GET", "/notes/anything", None).await;

    assert_eq!(fetched.status, 403, "{:?}", fetched.body);
}

#[tokio::test]
async fn a_denied_listing_is_a_403_not_an_empty_collection() {
    let app = app_with(PolicySet::new());

    let listed = send(&app, "GET", "/notes", None).await;

    assert_eq!(
        listed.status, 403,
        "failing closed means refusing, not returning nothing: {:?}",
        listed.body
    );
}

// --- Wiring ------------------------------------------------------------

#[tokio::test]
async fn an_unmounted_resource_is_a_startup_error_not_a_500() {
    let domain = Domain::builder().register::<Note>().permissive().build();

    // `MountedSchemas` is not `Debug`, so match rather than `expect_err`.
    let err = match MountedSchemas::new(&domain, [("widget", "/widgets")]) {
        Ok(_) => panic!("the domain registers no `widget`"),
        Err(err) => err,
    };

    assert!(
        err.contains("widget"),
        "the failure should name the resource: {err}"
    );
}

#[tokio::test]
async fn every_error_carries_the_requests_correlation_id() {
    use tower::ServiceExt;

    let request = axum::http::Request::builder()
        .method("GET")
        .uri("/notes/no-such-id")
        .header("x-request-id", "01H9K2QZ8V")
        .body(axum::body::Body::empty())
        .unwrap();

    let response = app().oneshot(request).await.unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(
        body["errors"][0]["id"], "01H9K2QZ8V",
        "this is what ties the response back to the audit log: {body}"
    );
}
