//! What is wrong, for the question every plugin system is asked most: *it is installed and nothing
//! happens.*
//!
//! With a path rather than a directory, that question is nearly always the same one — **the plugin
//! is not on the path**, or it is on it in a place that is not read. So the report leads with the
//! path itself and what would run from it, in order.
//!
//! Modelled on `vim.health`, which exists in neovim for the same reason: a check a *plugin* writes
//! is the only one that knows what that plugin needs — whether `age` is installed, whether its
//! database is writable — and no amount of checking by the shell can guess it.

use crate::runtimepath::{self, PluginFile, Root};

/// How one check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ok,
    /// It works, but something about it is worth saying.
    Warn,
    /// It does not work.
    Bad,
}

/// One line of a report.
#[derive(Debug, Clone)]
pub struct Finding {
    pub plugin: String,
    pub state: State,
    pub says: String,
}

impl Finding {
    fn new(plugin: &str, state: State, says: impl Into<String>) -> Finding {
        Finding {
            plugin: plugin.to_string(),
            state,
            says: says.into(),
        }
    }
}

/// The path, and what would run from it.
///
/// **Static checks only, in this pass.** Nothing here loads a plugin: a report should be safe to ask
/// for when a plugin is the thing misbehaving, and loading is what a plugin's own health check needs
/// — see [`self::checks_from`].
pub fn report() -> Vec<Finding> {
    let mut found = Vec::new();

    if !runtimepath::enabled() {
        found.push(Finding::new(
            "",
            State::Warn,
            "plugins are switched off for this shell (--noplugin, or OSLO_NOPLUGIN)",
        ));
    }

    // A root that is not there is the ordinary case, not a fault: the path is a list of places to
    // look, and most of them are empty on most machines. Only the ones that exist are worth a line.
    let roots = runtimepath::roots();
    for root in roots.iter().filter(|root| root.path.is_dir()) {
        found.push(Finding::new(
            "runtimepath",
            State::Ok,
            root.path.display().to_string(),
        ));
    }

    let files = runtimepath::plugin_files(&roots);
    if files.is_empty() {
        found.push(Finding::new(
            "",
            State::Ok,
            "nothing on the path to run — a plugin is a directory with plugin/*.lua in it",
        ));
    }
    for file in &files {
        found.push(Finding::new(
            &name_of(file),
            State::Ok,
            file.path.display().to_string(),
        ));
    }

    found.extend(stray_lua(&roots));
    found
}

fn name_of(file: &PluginFile) -> String {
    file.root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `.lua` files sitting in a root but in neither `plugin/` nor `lua/`.
///
/// The commonest way to put a plugin somewhere that is not read: dropping `thing.lua` straight into
/// the root, where nothing runs it and nothing requires it. Silence there is exactly the "installed
/// and nothing happens" the doctor exists for.
fn stray_lua(roots: &[Root]) -> Vec<Finding> {
    let mut found = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root.path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".lua") || name == "init.lua" {
                continue;
            }
            let root = root.path.display();
            found.push(Finding::new(
                "",
                State::Warn,
                format!(
                    "{} is run by nothing — move it to {root}/plugin/ to run it, \
                     or {root}/lua/ to require it",
                    path.display()
                ),
            ));
        }
    }
    found
}

/// What a plugin's own health checks answered, having loaded it to ask.
///
/// **This loads the plugin, and says so at the call site.** A check a plugin writes can only run
/// once its Lua has; that is the whole difference between this and [`report`], which is why they are
/// two functions and not one with a flag.
pub fn checks_from(name: &str) -> Vec<Finding> {
    let roots = runtimepath::roots();
    let Some(root) = roots
        .iter()
        .find(|root| root.path.file_name().is_some_and(|it| it == name))
    else {
        return vec![Finding::new(name, State::Bad, "not on the runtimepath")];
    };
    if let Err(problem) = super::load_from(&root.path) {
        return vec![Finding::new(name, State::Bad, problem)];
    }
    super::health::run(name)
        .into_iter()
        .map(|(state, says)| Finding::new(name, state, says))
        .collect()
}

#[cfg(test)]
#[path = "doctor/tests.rs"]
mod tests;
