//! Specs a config or a plugin declared, registered from above and consulted from here.
//!
//! The same inversion as `data::custom` and the hook registry: the table lives where it is *read*,
//! and the Lua API — which sits above the UI — puts things in it. Without that, `oslo-ui` would have
//! to know about Lua to offer a completion somebody wrote in Lua.
//!
//! # Why not a field on `SpecRegistry`
//!
//! The registry is built once, when the interactive loop starts. A plugin loads *later*, the first
//! time a line mentions one of its names, and a spec it registers then would arrive after the only
//! moment a field could have been filled. A plugin whose completions worked in the second session
//! and not the first is exactly the kind of bug nobody reports.
//!
//! Thread-local, because only the shell's own thread completes anything.

use super::CommandSpec;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    static ADDED: RefCell<HashMap<String, Rc<CommandSpec>>> = RefCell::new(HashMap::new());
}

/// Declare a spec for `spec.name`, replacing any earlier one under that name.
///
/// Replacing rather than refusing: a config that is edited and re-sourced would otherwise keep the
/// first version of everything it declared, which makes a config impossible to iterate on.
pub fn register(spec: CommandSpec) {
    ADDED.with(|slot| {
        slot.borrow_mut().insert(spec.name.clone(), Rc::new(spec));
    });
}

/// What was declared for `name`, if anything.
pub fn find(name: &str) -> Option<Rc<CommandSpec>> {
    ADDED.with(|slot| slot.borrow().get(name).cloned())
}

/// Every command something has declared a spec for.
pub fn declared() -> Vec<String> {
    let mut names: Vec<String> = ADDED.with(|slot| slot.borrow().keys().cloned().collect());
    names.sort();
    names
}

/// Forget everything declared. For tests, and for a config that is being reloaded.
pub fn forget() {
    ADDED.with(|slot| slot.borrow_mut().clear());
}

/// Drop the spec declared for `name`, answering whether there was one.
pub fn forget_named(name: &str) -> bool {
    ADDED.with(|slot| slot.borrow_mut().remove(name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> CommandSpec {
        CommandSpec {
            name: name.to_string(),
            description: "a test".to_string(),
            subcommands: Vec::new(),
            options: Vec::new(),
        }
    }

    #[test]
    fn what_was_declared_comes_back() {
        forget();
        register(spec("notes"));
        assert_eq!(
            find("notes").map(|s| s.name.clone()).as_deref(),
            Some("notes")
        );
        assert!(find("nothing").is_none());
        assert_eq!(declared(), vec!["notes".to_string()]);
        forget();
        assert!(find("notes").is_none());
    }

    /// **A config being re-sourced must not keep the old one.** Declaring twice is editing, not a
    /// conflict.
    #[test]
    fn the_second_declaration_replaces_the_first() {
        forget();
        register(spec("notes"));
        let mut second = spec("notes");
        second.description = "the new one".to_string();
        register(second);
        assert_eq!(find("notes").unwrap().description, "the new one");
        assert_eq!(declared().len(), 1);
        forget();
    }
}
