use ash_jsonapi::crud::Validators;
use ash_jsonapi::hook::Hooks;

#[derive(Clone)]
struct Plain;
impl Validators for Plain {}

#[derive(Clone)]
struct Hooked {
    hooks: Hooks,
}
impl Validators for Hooked {
    fn hooks(&self) -> Option<&Hooks> {
        Some(&self.hooks)
    }
}

#[test]
fn hooks_are_off_by_default() {
    assert!(
        Plain.hooks().is_none(),
        "a state that says nothing runs no hooks"
    );
}

#[test]
fn a_state_can_opt_in() {
    let app = Hooked {
        hooks: Hooks::new(),
    };
    assert!(
        app.hooks().is_some(),
        "overriding must be possible, not conflicting"
    );
}
