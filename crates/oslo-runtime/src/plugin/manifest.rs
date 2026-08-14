//! `plugin.lua` — what a plugin says it is, before any of it runs.
//!
//! ```lua
//! return {
//!   name     = "notes",
//!   version  = "0.1.0",
//!   entry    = "init.lua",
//!   builtins = { "note" },
//!   tools    = { "notes" },
//! }
//! ```
//!
//! # A table, evaluated in an interpreter of its own
//!
//! It is Lua because oslo is, and writing a manifest in JSON in a Lua-first shell would be a small
//! daily insult. It is evaluated in a **fresh [`Interp`] with no `oslo` global**, so a manifest that
//! tries to do something — register a builtin, open a database, read a file — finds nothing to do it
//! with. Reading what a plugin *claims* must not be the moment its code first runs.
//!
//! That is also why `install` can read one before you have decided to trust it.

use oslo_lua::value::Value;
use oslo_lua::{Interp, parse};
use std::path::Path;

/// The file a plugin directory must contain.
pub const FILE: &str = "plugin.lua";

/// What a plugin declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    /// The Lua file to run on first use, relative to the plugin's directory.
    pub entry: String,
    /// Builtin names to reserve — `note`, run as a command.
    pub builtins: Vec<String>,
    /// Row-producing tool names to reserve.
    pub tools: Vec<String>,
    /// The oldest oslo this plugin will run on, as written: `">= 0.2.29"` or `"0.2.29"`.
    ///
    /// `None` when the plugin did not say, which means it runs anywhere.
    pub requires: Option<String>,
    /// The user's secrets this plugin says it will read, by name.
    ///
    /// **A disclosure, not a sandbox.** It is shown at install, before trust is decided, and it is
    /// what `oslo.secret` filters a plugin's handle to. Nothing stops a plugin shelling out to
    /// `oslo secret get` instead — what the declaration buys is a plugin that can be caught
    /// contradicting its own manifest.
    pub secrets: Vec<String>,
    /// A hook whose firing loads this plugin, whether or not anything named it.
    ///
    /// **For a plugin that has nothing to be typed.** A plugin whose whole job is a `post-change-dir`
    /// handler would otherwise never load: loading happens when a line mentions one of its declared
    /// names, and nobody types the name of a plugin that only watches. `load_on = "post-change-dir"`
    /// is how it says so.
    pub load_on: Option<String>,
}

/// Does this oslo satisfy `requirement`?
///
/// **Only a minimum, and deliberately.** A plugin knows what it needs — `oslo.db` arrived in some
/// version and calling it in an older shell is a nil index — but it cannot know what a *later* oslo
/// will break, so an upper bound would be a guess that goes stale and locks working plugins out of
/// new releases. `>= X.Y.Z` and a bare `X.Y.Z` mean the same thing; anything else is refused when
/// the manifest is read, rather than silently treated as "runs anywhere".
pub fn satisfied(requirement: &str, version: &str) -> Result<bool, String> {
    let wanted = requirement.trim();
    let wanted = wanted.strip_prefix(">=").unwrap_or(wanted).trim();
    let least = oslo_base::version::numbers(wanted)
        .ok_or_else(|| format!("{requirement:?} is not a version, or a `>=` and a version"))?;
    let have = oslo_base::version::numbers(version)
        .ok_or_else(|| format!("{version:?} is not a version"))?;
    Ok(have >= least)
}

impl Manifest {
    /// Every name this plugin wants, builtins and tools together.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.builtins.iter().chain(self.tools.iter())
    }
}

/// Read and validate `<directory>/plugin.lua`.
pub fn read(directory: &Path) -> Result<Manifest, String> {
    let path = directory.join(FILE);
    let source =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let ast = parse(&source).map_err(|error| format!("{}: {error}", path.display()))?;

    let interp = Interp::new(path.to_string_lossy().into_owned());
    let returned = interp
        .run_ast(&ast)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let Some(Value::Table(table)) = returned.first() else {
        return Err(format!("{}: must return a table", path.display()));
    };
    let table = table.borrow();

    let name = string(&table, "name")
        .ok_or_else(|| format!("{}: `name` must be a string", path.display()))?;
    if !oslo_base::store::valid_name(&name) {
        return Err(format!(
            "{}: {name:?} is not a plugin name: letters, digits, and `.` `_` `-` after the first",
            path.display()
        ));
    }
    // **The directory is the name.** Otherwise two plugins could claim one name, or one could
    // claim another's and be loaded in its place by an index that trusted the manifest.
    let directory_name = directory
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if directory_name != name {
        return Err(format!(
            "{}: declares the name {name:?} but sits in a directory called {directory_name:?}",
            path.display()
        ));
    }

    let entry = string(&table, "entry").unwrap_or_else(|| "init.lua".to_string());
    if entry.contains('/') || entry.contains('\\') || entry.starts_with('.') {
        return Err(format!(
            "{}: `entry` must be a file in the plugin's own directory, not {entry:?}",
            path.display()
        ));
    }
    if !directory.join(&entry).is_file() {
        return Err(format!(
            "{}: `entry` {entry:?} is not there",
            path.display()
        ));
    }

    let secrets = strings(&table, "secrets", &path)?;
    for name in &secrets {
        if name.contains('/') || name.contains('*') || name.starts_with('.') || name.is_empty() {
            return Err(format!(
                "{}: `secrets` takes names, never a path or a pattern: {name:?}",
                path.display()
            ));
        }
    }

    let builtins = names(&table, "builtins", &path)?;
    let tools = names(&table, "tools", &path)?;
    if builtins.is_empty() && tools.is_empty() && string(&table, "load_on").is_none() {
        return Err(format!(
            "{}: declares no builtins, no tools and no `load_on`, so nothing would ever load it",
            path.display()
        ));
    }

    // Checked here, when the manifest is read, so a requirement nobody can satisfy — a typo, a
    // range this does not understand — is a manifest error at install rather than a plugin that
    // quietly never loads.
    let requires = string(&table, "requires");
    if let Some(requirement) = &requires {
        satisfied(requirement, oslo_base::version::current())
            .map_err(|problem| format!("{}: `requires`: {problem}", path.display()))?;
    }

    // Checked here for the same reason `requires` is: a hook name nothing will ever fire is a
    // plugin that silently never loads, and a typo is the likeliest way to write one.
    let load_on = string(&table, "load_on");
    if let Some(hook) = &load_on
        && crate::lua::api::hooks::resolve(hook).is_none()
    {
        return Err(format!(
            "{}: `load_on`: {hook:?} is not a hook; see `oslo hook`",
            path.display()
        ));
    }

    Ok(Manifest {
        name,
        version: string(&table, "version").unwrap_or_else(|| "0".to_string()),
        entry,
        builtins,
        tools,
        requires,
        secrets,
        load_on,
    })
}

/// One string field.
fn string(table: &oslo_lua::value::Table, field: &str) -> Option<String> {
    match table.get(&Value::str(field)) {
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

/// A list of plain strings.
fn strings(
    table: &oslo_lua::value::Table,
    field: &str,
    path: &Path,
) -> Result<Vec<String>, String> {
    let Value::Table(list) = table.get(&Value::str(field)) else {
        return Ok(Vec::new());
    };
    list.borrow()
        .sequence()
        .iter()
        .map(|value| match value {
            Value::Str(text) => Ok(text.to_string()),
            _ => Err(format!(
                "{}: every `{field}` entry must be a string",
                path.display()
            )),
        })
        .collect()
}

/// A list of command names, each of which must be one the shell could dispatch.
fn names(table: &oslo_lua::value::Table, field: &str, path: &Path) -> Result<Vec<String>, String> {
    let Value::Table(list) = table.get(&Value::str(field)) else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for value in list.borrow().sequence() {
        let Value::Str(name) = value else {
            return Err(format!(
                "{}: every `{field}` entry must be a string",
                path.display()
            ));
        };
        let name = name.to_string();
        if !usable_command_name(&name) {
            return Err(format!(
                "{}: {name:?} cannot be a command name",
                path.display()
            ));
        }
        found.push(name);
    }
    Ok(found)
}

/// Could the shell dispatch this word to a plugin?
///
/// Deliberately narrow. A name with a space, a slash or a `$` in it is not something anybody can
/// type as a command, and a name that is a shell keyword would be shadowed by the parser rather
/// than by the plugin — both of which look like the plugin silently not working.
fn usable_command_name(name: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "if", "then", "else", "elif", "fi", "case", "esac", "for", "while", "until", "do", "done",
        "function", "in", "select", "time", "coproc", "!", "{", "}", "[[", "]]",
    ];
    !name.is_empty()
        && name.len() <= 64
        && !KEYWORDS.contains(&name)
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c))
}

#[cfg(test)]
#[path = "manifest/tests.rs"]
mod tests;
