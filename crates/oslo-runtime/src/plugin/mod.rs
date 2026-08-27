//! Installing somebody else's Lua, and loading it the first time it is asked for.
//!
//! # Loaded by the loop, not by a stub
//!
//! The obvious design is a stub builtin per declared name, which loads the plugin when called. It
//! does not work, and the shell says so plainly:
//!
//! ```text
//! oslo: note: shell state is busy; an oslo.* call that reaches the shell cannot run from here.
//! ```
//!
//! A builtin runs *while the shell holds its state*, and `oslo.register_builtin` needs that state to
//! register anything — so a plugin loaded from inside a builtin can never register the builtin it
//! was loaded to provide. The stub is not a fixable design; it is the wrong place.
//!
//! So the loading happens in the read loop, one step before the line runs, where nothing is held:
//! [`crate::plugin::ensure_loaded`] is given the line, and any plugin whose name appears is loaded
//! then. By the time the line executes, the real builtin is registered and the shell dispatches to
//! it like any other.
//!
//! **The consequence worth knowing**: a name is not reserved until something mentions it. `type
//! note` before `note` has ever been typed reports it as not found, because it *is* not found — the
//! plugin providing it has not run. Mentioning it is what loads it.
//!
//! # What loads plugins
//!
//! Only the interactive shell. `oslo -c` and a script never read `init.lua` and never read the
//! index either: a plugin extends the shell you type at, and a script that depended on one would
//! break for anybody who had not installed it.

pub mod doctor;
pub mod health;
pub mod index;
pub mod install;
pub mod loading;
pub mod manifest;
/// The assertions a plugin writes about itself, and the harness that runs them.
pub mod test;
pub mod trust;

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;

thread_local! {
    /// What is installed and not yet loaded, by the names each one answers to.
    ///
    /// Read once, at startup. A plugin installed *during* a session is not picked up by it, which
    /// is the same rule the config follows and for the same reason: a session is a thing you
    /// started, and changing what it is under itself is how a shell becomes hard to reason about.
    static PENDING: RefCell<Vec<index::Installed>> = const { RefCell::new(Vec::new()) };
}

/// Where plugins live: the same directory their databases do.
pub fn directory() -> Option<PathBuf> {
    oslo_base::store::directory()
}

/// Read the index, so the loop knows which words are worth loading a plugin for.
///
/// **After the config, deliberately.** A config that defines a builtin of its own keeps it: a name
/// something already answers to is dropped here, so `oslo.register_builtin("note", …)` in your own
/// file wins over a plugin that claimed `note`. Yours is the one you can edit.
pub fn start(taken: impl Fn(&str) -> bool) {
    let mut pending = Vec::new();
    let mut claimed: HashSet<String> = HashSet::new();
    let entries = index::read();
    // **The index is a copy of every manifest, and a hand-edited manifest does not update it.**
    // Which builtins a plugin claims, what it requires and when it loads all come from here, so a
    // `oslo.toml` edited in place leaves the shell acting on what the plugin used to say — and a
    // plugin that "stopped working until it was reinstalled" gets blamed on the shell. Saying so
    // rather than rebuilding: the index also records the hash the install was allowed under, and
    // silently re-recording it would be this loop deciding an edit is trusted.
    if index::is_stale(&entries) {
        oslo_base::messages::warn(
            "plugins",
            "a plugin's manifest is newer than the index — reinstall it for the change to take",
        );
    }
    for installed in entries {
        let conflicts: Vec<&String> = installed
            .names()
            .filter(|name| taken(name) || !claimed.insert((*name).clone()))
            .collect();
        if !conflicts.is_empty() {
            oslo_base::messages::warn(
                format!("plugin {}", installed.name),
                format!(
                    "{} is already taken",
                    conflicts
                        .iter()
                        .map(|name| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
            continue;
        }
        pending.push(installed);
    }
    // **A hook nothing is attached to is never even asked for its handlers.** `on-key` and the rest
    // are gated by a bitset so that not using a hook costs nothing, and a plugin waiting on one has
    // attached nothing yet — so without this it would sleep through the very moment it named.
    for installed in &pending {
        if let Some(hook) = &installed.load_on
            && let Some((index, _)) = crate::lua::api::hooks::resolve(hook)
        {
            crate::lua::api::hooks::attached(index);
        }
    }
    PENDING.with(|slot| *slot.borrow_mut() = pending);
}

/// Load any plugin this line mentions, before the line runs.
///
/// **Every word, not only the first.** `note x | wc -l`, `x && note y`, and a plugin command inside
/// a function body are all lines that need the plugin, and none of them has it at the front. The
/// cost of being generous is a plugin loaded because its name appeared as a filename — which is a
/// plugin the user installed, doing what it was installed to do, slightly early.
pub fn ensure_loaded(line: &str) {
    if PENDING.with(|slot| slot.borrow().is_empty()) {
        return;
    }
    let words: HashSet<&str> = line.split_whitespace().collect();
    if words.is_empty() {
        return;
    }
    // Taken out of the pending list *before* loading, so a plugin that fails to load is not retried
    // on every line for the rest of the session.
    let wanted: Vec<index::Installed> = PENDING.with(|slot| {
        let mut pending = slot.borrow_mut();
        let (wanted, keep) = pending
            .drain(..)
            .partition(|installed| installed.names().any(|name| words.contains(name.as_str())));
        *pending = keep;
        wanted
    });
    for installed in wanted {
        if let Err(problem) = load(&installed) {
            oslo_base::messages::error(format!("plugin {}", installed.name), problem);
        }
    }
}

/// Load one plugin by hand, for the doctor: everything `ensure_loaded` does for a name it saw in a
/// line, asked for directly.
pub fn load_one(installed: &index::Installed) -> Result<(), String> {
    load(installed)
}

/// Load a plugin straight out of a directory, without it being installed. For `oslo plugin test`.
///
/// **No trust check, and that is not a hole.** The gate exists so that code arriving from somewhere
/// else cannot run without being read; this path runs a directory the author named on their own
/// command line, which is the same trust as `oslo ./script.lua`. Anything else would mean installing
/// a plugin before being allowed to test it.
pub fn load_from(directory: &std::path::Path) -> Result<manifest::Manifest, String> {
    let manifest = manifest::read(directory)?;
    loading::while_loading(&manifest.name, &manifest.secrets, || {
        crate::lua::engine::load_plugin_file(&directory.join(&manifest.entry))
    })?;
    Ok(manifest)
}

/// Load every plugin waiting on a hook that is about to fire.
///
/// **For a plugin with nothing to be typed.** [`ensure_loaded`] loads what a line *names*, which is
/// no use to a plugin whose whole job is a `post-change-dir` handler: nobody types the name of a
/// plugin that only watches, so it would sit in the index forever. `load_on = "post-change-dir"` in
/// its manifest is how it says which moment to wake up for.
///
/// Called from the loop at the same safe point everything else is, and *before* the hook fires — so
/// the handler the plugin registers is attached in time to hear the very firing that loaded it.
pub fn load_for_hook(hook: &str) {
    if PENDING.with(|slot| slot.borrow().is_empty()) {
        return;
    }
    let wanted: Vec<index::Installed> = PENDING.with(|slot| {
        let mut pending = slot.borrow_mut();
        let (wanted, keep) = pending
            .drain(..)
            .partition(|installed| installed.load_on.as_deref() == Some(hook));
        *pending = keep;
        wanted
    });
    for installed in wanted {
        if let Err(problem) = load(&installed) {
            oslo_base::messages::error(format!("plugin {}", installed.name), problem);
        }
    }
}

/// Every hook some installed plugin is waiting on, so the loop can ask cheaply.
pub fn hooks_waited_on() -> Vec<String> {
    PENDING.with(|slot| {
        slot.borrow()
            .iter()
            .filter_map(|installed| installed.load_on.clone())
            .collect()
    })
}

/// Check the plugin still hashes to what was allowed, then run its entry file.
fn load(installed: &index::Installed) -> Result<(), String> {
    let directory = installed
        .directory()
        .ok_or_else(|| "nowhere to look for it".to_string())?;
    // **Checked at load as well as at install**, because the shell it was installed against is not
    // necessarily the shell running now: downgrading oslo, or copying a home to an older machine,
    // both leave a plugin recorded as fine and unable to work.
    if let Some(requirement) = &installed.requires
        && !manifest::satisfied(requirement, oslo_base::version::current())?
    {
        return Err(format!(
            "it needs oslo {requirement} and this is {}",
            oslo_base::version::current()
        ));
    }
    if !trust::unchanged(&directory, &installed.hash)? {
        return Err(format!(
            "it has changed since you allowed it; run `oslo plugin allow {}` \
             after looking at what changed",
            installed.name
        ));
    }
    // Attributed while it runs, so a handle `oslo.secret` hands out during the load knows which
    // plugin asked for it.
    loading::while_loading(&installed.name, &installed.secrets, || {
        crate::lua::engine::load_plugin_file(&directory.join(&installed.entry))
    })
}

#[cfg(test)]
#[path = "mod/tests.rs"]
mod tests;
