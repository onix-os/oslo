//! The stored macros, brought into a shell — at startup, and again whenever they change.
//!
//! # After the config, deliberately
//!
//! Three sources define an alias: `alias` in a script, `oslo.alias` in `config.lua`, and
//! `oslo macros add`. The ordinary shell rule is that the last definition wins, and this applies it
//! to sources: **the database is applied after the configuration, so the database wins.**
//!
//! That is the deliberate half. The database is the one you can change without editing a file, so
//! `oslo macros add gco …` taking effect is what you asked for; a config that has to be edited and
//! re-sourced would be the more surprising winner. The cost is that a stored entry can shadow one
//! you wrote in `config.lua` — which is why what the config defined is written out here, so that the
//! manager can show you both, and so that *removing* the stored one puts the configured one back
//! rather than leaving a hole.
//!
//! # From the snapshot, not the database
//!
//! Measured: opening the database is 2.6 ms against an 842 µs shell start, and reading the flat file
//! is 3.6 µs. See `oslo_base::macros`. A shell that opened a second key-value store before its
//! first prompt would be paying three times its whole startup for something a `read(2)` answers.
//!
//! # And only where the config is read
//!
//! This runs from the interactive loop, beside `load_config`, and from nowhere else — so a script or
//! an `oslo -c` sees none of it. That is not a new restriction: `config.lua` is read by this loop and
//! by nothing else either, so a non-interactive shell has never had aliases to expand.

use oslo_base::macros::live::{Applied, Stamps};
use oslo_base::macros::{Entry, Kind};
use oslo_shell::env::Environment;
use std::sync::{Arc, Mutex};

/// What this shell was last given, so the next rebuild knows what is its to take away.
pub(super) struct Held {
    applied: Applied,
    stamps: Stamps,
}

/// Apply what `oslo macros` has stored, having first written down what the config defined.
pub(super) fn install(env: &Arc<Mutex<Environment>>) -> Held {
    // **Written before anything is applied over it**, which is the only moment the configuration's
    // own aliases can be told apart from everybody else's.
    publish_what_the_config_defined(env);
    // A session file outlives the shell that wrote it; a starting shell is where they get tidied.
    oslo_base::macros::live::session::sweep();

    let wanted = oslo_base::macros::live::want();
    let applied = Applied::of(&wanted);
    apply(env, &wanted, &Applied::default(), &applied);
    Held {
        applied,
        stamps: Stamps::now(),
    }
}

/// Bring the shell back in step if either file has moved since the last prompt.
///
/// Two `stat`s and, almost always, nothing else — the shape `universal::refresh` already has, and
/// for the same reason: a change made in one terminal has to reach the one beside it, and reading
/// the files every prompt to find out would cost more than the feature is worth.
pub(super) fn refresh(env: &Arc<Mutex<Environment>>, held: Held) -> Held {
    let stamps = Stamps::now();
    if stamps == held.stamps {
        return held;
    }
    let wanted = oslo_base::macros::live::want();
    let applied = Applied::of(&wanted);
    apply(env, &wanted, &held.applied, &applied);
    Held { applied, stamps }
}

/// Take away what was ours and is not wanted any more, then put the wanted set in.
///
/// In that order, because a name in both lists must end up **set**: the other way round would add
/// `gs` and then remove it again for having been in yesterday's set too.
fn apply(env: &Arc<Mutex<Environment>>, wanted: &[Entry], had: &Applied, now: &Applied) {
    let gone = had.gone(now);
    for name in &gone.abbrevs {
        oslo_ui::abbr::remove(name);
    }
    let Ok(mut guard) = env.lock() else {
        return;
    };
    for name in &gone.aliases {
        guard.remove_alias(name);
    }
    // The recipe, not the value: see `Applied::gone`. A variable already read is a variable like
    // any other by now, and unsetting it under a running command would be a different feature.
    for name in &gone.vars {
        guard.remove_lazy_var(name);
    }
    for entry in wanted {
        match entry.kind {
            Kind::Alias => guard.set_alias(&entry.name, &entry.body),
            Kind::Abbrev => oslo_ui::abbr::add(
                &entry.name,
                &entry.body,
                // The placement a stored abbreviation gets. `oslo macros` has no `--anywhere` yet,
                // and command position is what an abbreviation is for; widening it later is adding
                // a flag rather than changing what these mean.
                oslo_ui::abbr::Placement::Command,
            ),
            // **A value now; a recipe when somebody asks.**
            //
            // A plain value costs nothing, so it is exported here like any other environment
            // variable — which is also the only way a program that reads the environment *itself*
            // can find it, since nothing about `gh` or `aws` expands `$NAME` first.
            //
            // A body that runs a command is different in kind: doing that for every stored variable
            // at every prompt would decrypt everything you own to answer a question nobody asked.
            // The shell is handed the line and runs it the first time the name is read — see
            // `oslo_shell::expand::param::materialise`.
            // **Neither overrules the environment this shell was started with**, which is the same
            // rule the lazy path follows and the reason `FOO=x oslo …` still means something.
            Kind::Var if guard.get_var(&entry.name).is_some() => {}
            Kind::Var if oslo_base::macros::is_a_value(&entry.body) => {
                guard.set_var(&entry.name, entry.body.trim(), true);
            }
            Kind::Var => guard.set_lazy_var(&entry.name, &entry.body),
            // A function or a script is found after `$PATH` fails, so neither is in the snapshot
            // and neither belongs here. See `oslo_shell::exec::stored`.
            Kind::Func | Kind::Script => {}
        }
    }
}

/// Write down the aliases and abbreviations the *configuration* defined.
///
/// The manager's second source, and what makes a removal able to put the configured alias back.
/// Everyone else reads it from the file rather than by running Lua, because this is the only
/// process that has already run it.
fn publish_what_the_config_defined(env: &Arc<Mutex<Environment>>) -> Option<()> {
    let mut entries: Vec<Entry> = {
        let guard = env.lock().ok()?;
        guard
            .get_aliases()
            .iter()
            .map(|(name, body)| Entry::new(Kind::Alias, name, body))
            .collect()
    };
    entries.extend(
        oslo_ui::abbr::all()
            .into_iter()
            .map(|(name, abbr)| Entry::new(Kind::Abbrev, &name, &abbr.expansion)),
    );
    entries.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
    // Best effort: this is a list the manager draws. A shell that failed to start because it could
    // not write one would be absurd.
    oslo_base::macros::live::publish_elsewhere(&entries).ok()
}
