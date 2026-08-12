//! The stored aliases and abbreviations, brought into a starting shell.
//!
//! # After the config, deliberately
//!
//! Three sources define an alias: `alias` in a script, `oslo.alias` in `config.lua`, and
//! `oslo aliases add`. The ordinary shell rule is that the last definition wins, and this applies it
//! to sources: **the database is applied after the configuration, so the database wins.**
//!
//! That is the deliberate half. The database is the one you can change without editing a file, so
//! `oslo aliases add gco …` taking effect is what you asked for; a config that has to be edited and
//! re-sourced would be the more surprising winner. The cost is that a stored entry can shadow one
//! you wrote in `config.lua` — which is why the names the config defined are written out here, so
//! that `oslo aliases show` can mark it rather than leave you wondering why your config stopped
//! working.
//!
//! # From the snapshot, not the database
//!
//! Measured: opening the database is 2.6 ms against an 842 µs shell start, and reading the flat file
//! is 3.6 µs. See `oslo_base::aliases`. A shell that opened a second key-value store before its
//! first prompt would be paying three times its whole startup for something a `read(2)` answers.
//!
//! # And only where the config is read
//!
//! This runs from the interactive loop, beside `load_config`, and from nowhere else — so a script or
//! an `oslo -c` sees none of it. That is not a new restriction: `config.lua` is read by this loop and
//! by nothing else either, so a non-interactive shell has never had aliases to expand.

use oslo_base::aliases::{Kind, snapshot};
use oslo_shell::env::Environment;
use std::sync::{Arc, Mutex};

/// Apply what `oslo aliases` has stored, and record what the config had defined first.
pub(super) fn install(env: &Arc<Mutex<Environment>>) {
    let entries = snapshot::read();
    // Written whether or not anything is stored, so that removing the last entry updates the file
    // rather than leaving yesterday's answer in it.
    let configured = record_configured(env);
    if entries.is_empty() {
        return;
    }

    let Ok(mut guard) = env.lock() else {
        return;
    };
    for entry in &entries {
        match entry.kind {
            Kind::Alias => guard.set_alias(&entry.name, &entry.body),
            Kind::Abbrev => oslo_ui::abbr::add(
                &entry.name,
                &entry.body,
                // The placement a stored abbreviation gets. `oslo aliases` has no `--anywhere` yet,
                // and command position is what an abbreviation is for; widening it later is adding
                // a flag rather than changing what these mean.
                oslo_ui::abbr::Placement::Command,
            ),
            // A function or a script is found after `$PATH` fails, so neither is in the snapshot
            // and neither belongs here. See `exec::stored`.
            Kind::Func | Kind::Script => {}
        }
    }
    drop(guard);
    let _ = configured;
}

/// Write down which alias names the *configuration* defined, before the database overwrites any.
///
/// A label for `oslo aliases show`, so it can say "this shadows config.lua". Read from a file rather
/// than by running Lua, because drawing a label is not worth starting an interpreter for.
fn record_configured(env: &Arc<Mutex<Environment>>) -> Option<()> {
    let names: Vec<String> = {
        let guard = env.lock().ok()?;
        guard.get_aliases().keys().cloned().collect()
    };
    let dir = oslo_base::aliases::directory()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("configured.names");
    let mut text = names.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    // Best effort: this is a label on a list. A shell that failed to start because it could not
    // write one would be absurd.
    std::fs::write(path, text).ok()
}
