//! `registry!` must support several resources, and validators alongside hooks.
use ash_jsonapi::crud::Validators;
use ash_jsonapi::hook::Hooks;
use ash_jsonapi::validation::DocumentValidator;
use std::sync::Arc;

#[derive(Clone)]
struct Todo {
    id: String,
    message: String,
}
#[derive(Clone)]
struct User {
    id: String,
    email: String,
}

/// Carries validators *and* hooks — the shape that was impossible before,
/// because `registry!` wrote the whole `Validators` impl.
#[derive(Clone)]
struct App {
    todos: Arc<DocumentValidator>,
    users: Arc<DocumentValidator>,
    hooks: Hooks,
}

ash_jsonapi::resource! {
    Todo as "todo" at "/todos",
    state: App, schema: TodoSchema, input: NewTodo, id: id,
    attributes: { message: String },
}
ash_jsonapi::resource! {
    User as "user" at "/users",
    state: App, schema: UserSchema, input: NewUser, id: id,
    attributes: { email: String },
}

ash_jsonapi::registry! { App { todos: Todo, users: User } hooks: hooks }

fn app() -> App {
    App {
        todos: Arc::new(Todo::validator().unwrap()),
        users: Arc::new(User::validator().unwrap()),
        hooks: Hooks::new(),
    }
}

#[test]
fn several_resources_are_looked_up_by_name() {
    let s = app();
    assert!(s.validator("todo").is_some());
    assert!(s.validator("user").is_some());
    assert!(s.validator("nope").is_none());
}

#[test]
fn hooks_and_validators_coexist() {
    assert!(app().hooks().is_some(), "the hooks arm must wire hooks too");
}
