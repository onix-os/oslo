//! `make <Tab>` offers the recipes this project actually has.
//!
//! ```console
//! $ make <Tab>
//!   build     the release binary          verify    the whole local gate
//!   test      the suite                   configs   install config/ into …
//! ```
//!
//! # Nothing is generated, and nothing has to be kept in step
//!
//! Every other completion in this tree is a *description* of a command written down somewhere — a
//! spec file, a config's `oslo.completion.spec`, an argc comment. This one is not: `.make.lua` is
//! read by the same shell that is completing the line, and every `make.recipe{ name = … }` in it is
//! a name the user is about to type. **The recipe table is the spec.** A recipe added five seconds
//! ago is on offer, with its `desc` as the description and its `params` as its flags, and there is
//! no file to install and nothing that can drift.
//!
//! # Reading it is safe, and that is a property of the format rather than a hope
//!
//! `make.recipe` *declares*: it stores `run` as a function and calls nothing. Loading a `.make.lua`
//! therefore runs its top level and no recipe body — which is what `oslo make --list` has always
//! relied on, and is why this can happen on a keystroke at all.
//!
//! Two things it deliberately does not do. It does not `chdir`, because the shell being completed
//! for is standing somewhere and a completion may not move it — so a file whose top level globs
//! against the project resolves against the wrong directory, fails, and answers nothing rather than
//! wrongly. And it does not load `init.lua`: a recipe file that needs a person's own helpers at its
//! *top level* is one this declines, where `oslo make` itself would still run it.
//!
//! # Once per file, not once per keystroke
//!
//! An engine and a file read are far too much to pay per Tab, so the answer is kept against the
//! file's path and modification time. Editing `.make.lua` changes the time and the next Tab pays
//! again; standing still costs one `stat` of each ancestor.

use oslo_base::value::Value;
use oslo_ui::spec::{Arg, CommandSpec, OptionSpec, SubcommandSpec};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

thread_local! {
    /// The file this was worked out from, when it was last written, and the answer.
    static CACHED: RefCell<Option<(PathBuf, Option<SystemTime>, CommandSpec)>> =
        const { RefCell::new(None) };
}

/// Install it. Answers for `make` and for nothing else.
pub(super) fn register() {
    oslo_ui::spec::custom::add_source(
        "recipes",
        std::rc::Rc::new(|command: &str| match command {
            "make" => spec(),
            _ => None,
        }),
    );
}

/// The spec for the `.make.lua` governing the working directory, if there is one.
fn spec() -> Option<CommandSpec> {
    let here = std::env::current_dir().ok()?;
    let file = oslo_shell::make::governing(&here)?;
    let stamp = std::fs::metadata(&file).and_then(|m| m.modified()).ok();

    let cached = CACHED.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|(path, had, _)| path == &file && had == &stamp)
            .map(|(_, _, spec)| spec.clone())
    });
    if let Some(spec) = cached {
        return Some(spec);
    }

    let built = read(&file)?;
    CACHED.with(|slot| *slot.borrow_mut() = Some((file, stamp, built.clone())));
    Some(built)
}

/// Load the file in an engine of its own and ask it what it declared.
fn read(file: &Path) -> Option<CommandSpec> {
    let engine = crate::LuaEngine::new().ok()?;
    let env = std::sync::Arc::new(std::sync::Mutex::new(oslo_shell::env::Environment::new()));
    if !crate::startup::lua_init::install_bindings(&engine, env) {
        return None;
    }
    engine.load_file(file.to_str()?).ok()?;
    // `names()` is data — the declarations, never the bodies. It has existed since the runner did,
    // for exactly this, and until now nothing called it.
    //
    // Parked on the table rather than returned, because `eval_as` answers a status and not a value.
    // What comes back from `oslo_table` is a copy, which is all this wants: it is read once, here.
    engine
        .eval_as("oslo.make.__names = oslo.make.names()", "oslo.make")
        .ok()?;
    Some(from_names(&field(
        &engine.oslo_table(),
        &["make", "__names"],
    )?))
}

/// Walk a path of keys into a table.
fn field(value: &Value, path: &[&str]) -> Option<Value> {
    let mut at = value.clone();
    for key in path {
        let Value::Table(table) = at else {
            return None;
        };
        at = table.borrow().get_str(key);
    }
    Some(at)
}

/// What `oslo.make.names()` answered, as a spec.
fn from_names(value: &Value) -> CommandSpec {
    let Value::Table(list) = value else {
        return CommandSpec::default();
    };
    let subcommands: Vec<SubcommandSpec> = list
        .borrow()
        .sequence()
        .iter()
        .filter_map(|entry| {
            let Value::Table(recipe) = entry else {
                return None;
            };
            let recipe = recipe.borrow();
            let name = string(&recipe, "name")?;
            // A `_`-prefixed recipe is runnable and left out of the listing, which is the same
            // thing as being left out of the menu.
            if recipe.get_str("private").truthy() {
                return None;
            }
            Some(SubcommandSpec {
                name,
                description: string(&recipe, "desc").unwrap_or_default(),
                options: params(&recipe),
                ..SubcommandSpec::default()
            })
        })
        .collect();

    CommandSpec {
        name: "make".to_string(),
        description: "run a recipe from this project's .make.lua".to_string(),
        subcommands,
        ..CommandSpec::default()
    }
}

/// One recipe's declared parameters, as flags.
fn params(recipe: &oslo_base::value::Table) -> Vec<OptionSpec> {
    let Value::Table(list) = recipe.get_str("params") else {
        return Vec::new();
    };
    list.borrow()
        .sequence()
        .iter()
        .filter_map(|entry| {
            let Value::Table(param) = entry else {
                return None;
            };
            let param = param.borrow();
            let name = string(&param, "name")?;
            Some(OptionSpec {
                names: vec![name],
                description: string(&param, "desc").unwrap_or_default(),
                takes: match param.get_str("takes_value").truthy() {
                    true => Arg::Required,
                    false => Arg::None,
                },
                ..OptionSpec::default()
            })
        })
        .collect()
}

fn string(table: &oslo_base::value::Table, key: &str) -> Option<String> {
    match table.get_str(key) {
        Value::Str(text) => Some(text.to_string()).filter(|t| !t.is_empty()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "recipes/tests.rs"]
mod tests;
