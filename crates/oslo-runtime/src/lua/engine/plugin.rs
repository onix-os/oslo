//! Running a plugin's entry file on the session's own interpreter.
//!
//! **The session's interpreter, not a fresh one** — the opposite of how a manifest is read. A
//! manifest is data and is evaluated somewhere it can reach nothing; an entry file *is* the plugin,
//! and registering a builtin on this session is the whole reason it runs.
//!
//! Its own file because `engine.rs` is at the 600-line limit and this is a subject of its own: the
//! engine is what the shell talks to Lua through, and this is one caller of it.

use super::ACTIVE;
use std::path::Path;

/// How much a plugin's entry file may add to the VM's heap before it is stopped.
///
/// **A margin over what is already in use, not an absolute figure.** There is one heap and the
/// session's own config is already on it, so a fixed ceiling would be a limit on the config as much
/// as on the plugin. A plugin's *load* registers builtins and completions; 64 MiB is far more than
/// any of that and still catches the table that grows without end.
const HEADROOM: usize = 64 * 1024 * 1024;

/// Read `path` and run it, answering why not if it could not.
///
/// # Why there is a ceiling here and nowhere else
///
/// A plugin's entry file is somebody else's Lua, and it is the one chunk in the shell that runs on
/// this interpreter without having been written by the person at the prompt. It also has an *end* —
/// registering what it registers and returning — which is what makes a ceiling meaningful: it can
/// be taken off again afterwards.
///
/// **It is not a sandbox and does not pretend to be.** The plugin's hooks and callbacks run later,
/// with no ceiling, and any of them can start a command. See `docs/features/plugins.md` on why a
/// disclosure rather than a boundary. What this stops is a load that would take the shell down with
/// it — the runaway table, not the hostile author.
/// `root` reaches the chunk as `...` -- Lua's own way of telling a chunk where it lives. A plugin
/// shipping a script or a data file beside its `plugin/` has no other way to name it, and hardcoding
/// the install path breaks the moment XDG_DATA_HOME moves. hexe and trek hand it over the same way.
pub(crate) fn load_plugin_file(path: &Path, root: &Path) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let name = path.to_string_lossy().into_owned();
    let Some((interp, _)) = ACTIVE.with(|slot| slot.borrow().clone()) else {
        return Err("no Lua interpreter on this thread".to_string());
    };

    let ceiling = interp.memory_used().map(|used| used + HEADROOM);
    let bounded = ceiling.is_some_and(|limit| interp.set_memory_limit(Some(limit)));

    // Named, so an error inside the plugin points at the plugin's file rather than at the last
    // chunk this interpreter happened to run.
    interp.set_varargs(vec![oslo_base::value::Value::str(root.to_string_lossy())]);
    let outcome = interp.eval(&source, &name);
    interp.set_varargs(Vec::new());

    // **Asked before the ceiling comes off**, because afterwards there is nothing to compare
    // against. The VM does report a stop as an error, but as one about an executor rather than
    // about memory; still being over the limit after the collector has been through twice is what
    // actually happened, and is what the person who installed the plugin needs told.
    let overrun = matches!(
        (ceiling, interp.memory_used()),
        (Some(limit), Some(used)) if bounded && used > limit
    );
    if bounded {
        interp.set_memory_limit(None);
    }
    if overrun {
        return Err(format!(
            "it was stopped part-way through loading: it asked for more than {} MB of memory",
            HEADROOM / (1024 * 1024)
        ));
    }
    outcome.map_err(|error| format!("{error}"))?;
    Ok(())
}
