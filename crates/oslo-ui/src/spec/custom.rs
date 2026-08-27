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
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

thread_local! {
    static ADDED: RefCell<HashMap<String, Rc<CommandSpec>>> = RefCell::new(HashMap::new());
    static LOADER: RefCell<Option<Loader>> = const { RefCell::new(None) };
    /// Names a loader has already been asked about and had nothing for.
    static MISSING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static SOURCES: RefCell<Vec<(String, Source)>> = const { RefCell::new(Vec::new()) };
}

/// A spec **worked out on demand** rather than declared once or read from a file.
///
/// `make` is the case that needs it: the recipes it should offer are the ones in the `.make.lua`
/// governing the directory you are standing in *now*, and a recipe added a second ago is a name you
/// are about to type. Nothing about that can be written down in advance, so nothing about it can be
/// cached here either — a source is asked on **every** lookup, and one that is expensive keeps its
/// own cache, because only it knows what would make its answer stale.
pub type Source = Rc<dyn Fn(&str) -> Option<CommandSpec>>;

/// Add a source, replacing any earlier one of the same name.
pub fn add_source(name: &str, source: Source) {
    SOURCES.with(|slot| {
        let mut sources = slot.borrow_mut();
        match sources.iter().position(|(had, _)| had == name) {
            Some(at) => sources[at] = (name.to_string(), source),
            None => sources.push((name.to_string(), source)),
        }
    });
    // A source can answer for a name a loader was once asked about and had nothing for.
    MISSING.with(|slot| slot.borrow_mut().clear());
}

/// The names of the sources installed, in the order they are asked.
pub fn sources() -> Vec<String> {
    SOURCES.with(|slot| slot.borrow().iter().map(|(name, _)| name.clone()).collect())
}

/// Somewhere a spec for a command might be found — a directory of `.yaml` files, say.
///
/// The same inversion as the rest of this module, one step further out: `oslo-ui` cannot read a
/// spec file, because a spec file holds macros and running one means the shell. So the shell says
/// where to look, and this decides *when* — once per name, on the first Tab that mentions it.
pub type Loader = Rc<dyn Fn(&str) -> Option<CommandSpec>>;

/// Install the place specs are looked for. `None` removes it.
pub fn set_loader(loader: Option<Loader>) {
    LOADER.with(|slot| *slot.borrow_mut() = loader);
    MISSING.with(|slot| slot.borrow_mut().clear());
}

/// Declare a spec for `spec.name`, replacing any earlier one under that name.
///
/// Replacing rather than refusing: a config that is edited and re-sourced would otherwise keep the
/// first version of everything it declared, which makes a config impossible to iterate on.
pub fn register(spec: CommandSpec) {
    let name = spec.name.clone();
    MISSING.with(|slot| slot.borrow_mut().remove(&name));
    ADDED.with(|slot| {
        slot.borrow_mut().insert(name, Rc::new(spec));
    });
}

/// What was declared for `name`, or what a loader can find for it.
///
/// **A miss is remembered too.** Without that, every keystroke against a command nobody wrote a
/// spec for costs a directory lookup — and the commands nobody wrote a spec for are most of them.
pub fn find(name: &str) -> Option<Rc<CommandSpec>> {
    if let Some(found) = ADDED.with(|slot| slot.borrow().get(name).cloned()) {
        return Some(found);
    }
    // **A computed source outranks a file.** `make` has a spec in the shipped corpus — GNU make's
    // flags — and in a directory with a `.make.lua` the recipes of *this project* are the better
    // answer by a distance. Where no source answers, the file is still there.
    //
    // Cloned out of the cell before calling, like the loader below: a source reads a file and may
    // end in Lua, which can complete another word and come back through here.
    let installed: Vec<Source> =
        SOURCES.with(|slot| slot.borrow().iter().map(|(_, s)| Rc::clone(s)).collect());
    for source in installed {
        if let Some(spec) = source(name) {
            return Some(Rc::new(spec));
        }
    }
    if MISSING.with(|slot| slot.borrow().contains(name)) {
        return None;
    }
    // Cloned out of the cell before calling: a loader reads a file, and reading a file can end in
    // Lua, which can complete another word and come back through here.
    let loader = LOADER.with(|slot| slot.borrow().clone())?;
    match loader(name) {
        Some(spec) => {
            let spec = Rc::new(spec);
            ADDED.with(|slot| slot.borrow_mut().insert(name.to_string(), Rc::clone(&spec)));
            Some(spec)
        }
        None => {
            MISSING.with(|slot| slot.borrow_mut().insert(name.to_string()));
            None
        }
    }
}

/// Every command something has declared a spec for.
///
/// What has been *declared*, which after a lookup includes what a loader found. A loader is a
/// directory, and listing it here would be listing the disk rather than the session.
pub fn declared() -> Vec<String> {
    let mut names: Vec<String> = ADDED.with(|slot| slot.borrow().keys().cloned().collect());
    names.sort();
    names
}

/// Forget everything declared. For tests, and for a config that is being reloaded.
pub fn forget() {
    ADDED.with(|slot| slot.borrow_mut().clear());
    MISSING.with(|slot| slot.borrow_mut().clear());
}

/// Drop the spec declared for `name`, answering whether there was one.
///
/// **And remember that there is none**, so a loader does not put the file back on the next
/// keystroke. Forgetting means "this command has no spec now"; re-declaring one lifts it.
pub fn forget_named(name: &str) -> bool {
    MISSING.with(|slot| slot.borrow_mut().insert(name.to_string()));
    ADDED.with(|slot| slot.borrow_mut().remove(name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> CommandSpec {
        CommandSpec {
            name: name.to_string(),
            description: "a test".to_string(),
            ..CommandSpec::default()
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

/// **A loader is asked once per name, and its refusal counts.** Every keystroke against a command
/// with no spec would otherwise be a directory lookup, and most commands have no spec.
#[cfg(test)]
mod loader_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_loader_answers_for_what_was_never_declared_and_is_asked_once() {
        forget();
        let asked = Rc::new(Cell::new(0usize));
        let counter = Rc::clone(&asked);
        set_loader(Some(Rc::new(move |name: &str| {
            counter.set(counter.get() + 1);
            (name == "found").then(|| CommandSpec {
                name: name.to_string(),
                description: "from a file".to_string(),
                ..CommandSpec::default()
            })
        })));

        assert_eq!(
            find("found").map(|s| s.description.clone()).as_deref(),
            Some("from a file")
        );
        assert!(find("found").is_some());
        assert_eq!(asked.get(), 1, "the answer is cached");

        assert!(find("absent").is_none());
        assert!(find("absent").is_none());
        assert_eq!(asked.get(), 2, "and so is the refusal");

        set_loader(None);
        forget();
    }

    /// A declared spec is not looked for, so a config's own always wins over a file.
    #[test]
    fn what_was_declared_is_never_looked_up() {
        forget();
        set_loader(Some(Rc::new(|_| {
            panic!("the loader must not be asked");
        })));
        register(CommandSpec {
            name: "notes".into(),
            description: "declared".into(),
            ..CommandSpec::default()
        });
        assert_eq!(
            find("notes").map(|s| s.description.clone()).as_deref(),
            Some("declared")
        );
        set_loader(None);
        forget();
    }
}
