//! Running a plugin's entry file on the session's own interpreter.
//!
//! **The session's interpreter, not a fresh one** — the opposite of how a manifest is read. A
//! manifest is data and is evaluated somewhere it can reach nothing; an entry file *is* the plugin,
//! and registering a builtin on this session is the whole reason it runs.
//!
//! Its own file because `engine.rs` is at the 600-line limit and this is a subject of its own: the
//! engine is what the shell talks to Lua through, and this is one caller of it.

use super::ACTIVE;
use crate::lua::eval;
use std::path::Path;

/// Read `path` and run it, answering why not if it could not.
pub(crate) fn load_plugin_file(path: &Path) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let name = path.to_string_lossy().into_owned();
    let ast = eval::parse(&source).map_err(|error| format!("{name}: {error}"))?;
    let Some((interp, _)) = ACTIVE.with(|slot| slot.borrow().clone()) else {
        return Err("no Lua interpreter on this thread".to_string());
    };
    // Named, so an error inside the plugin points at the plugin's file rather than at the last
    // chunk this interpreter happened to run.
    interp.set_chunk(name);
    interp.run_ast(&ast).map_err(|error| format!("{error}"))?;
    Ok(())
}
