//! Every hook has a fire site, and it is the right *kind* of fire site.
//!
//! # What this exists to catch
//!
//! `oslo.on.on_command_not_found(f)` shipped, was documented, and never fired. Handlers are stored
//! under a hook's canonical name; the one fire site asked for `"command-not-found"`, an alias. The
//! lookup found an empty list and the executor carried on to "command not found" as though nothing
//! were attached — which is exactly what a config with a typo in the hook name looks like, so
//! there was nothing to notice.
//!
//! `lua/api/hooks.rs` says a row on its own is caught "by the test that walks them". The test that
//! existed walked the *names*, and every name resolved perfectly. What nothing walked was whether
//! anything in the shell ever fires the moment those names point at.
//!
//! So this reads the source. Not elegant, and the right kind of ugly: the alternative is firing
//! twenty hooks through a line editor and a job reaper, and a test that cannot be written does not
//! catch anything.
//!
//! # The two rules
//!
//! 1. **Every hook is fired by something.** A row in `HOOKS` with no fire site is a promise the
//!    shell does not keep.
//! 2. **`Hook.answers` decides which dispatch it goes through.** A hook that answers must be fired
//!    by `answer_hook_with` or `ask_hook_here`, whose callers read the result; one that only
//!    observes must go through `fire_at_here`, which discards it and may defer it. Firing an
//!    answering hook through the notifying path would drop a veto on the floor.

use oslo::lua::api::hooks::{HOOKS, at};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn source_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `src/`, as one string per file.
fn sources() -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    let mut stack = vec![source_dir()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text =
                    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
                found.push((path, text));
            }
        }
    }
    assert!(found.len() > 50, "the source walk found almost nothing");
    found
}

/// The `at::` constant names, by the index each one holds.
///
/// Built from the constants themselves rather than from a second list, so a renamed constant
/// cannot leave this test looking at a name nothing uses.
fn constant_names() -> HashMap<usize, &'static str> {
    HashMap::from([
        (at::PRE_CMD, "PRE_CMD"),
        (at::POST_CMD, "POST_CMD"),
        (at::PRE_CHANGE_DIR, "PRE_CHANGE_DIR"),
        (at::POST_CHANGE_DIR, "POST_CHANGE_DIR"),
        (at::PRE_PROMPT, "PRE_PROMPT"),
        (at::POST_PROMPT, "POST_PROMPT"),
        (at::PRE_MODE_CHANGE, "PRE_MODE_CHANGE"),
        (at::POST_MODE_CHANGE, "POST_MODE_CHANGE"),
        (at::HISTORY_OPEN, "HISTORY_OPEN"),
        (at::HISTORY_CLOSE, "HISTORY_CLOSE"),
        (at::HISTORY_SELECT, "HISTORY_SELECT"),
        (at::COMPLETION_START, "COMPLETION_START"),
        (at::COMPLETION_CANCEL, "COMPLETION_CANCEL"),
        (at::COMPLETION_SELECT, "COMPLETION_SELECT"),
        (at::JOB_FINISH, "JOB_FINISH"),
        (at::TIME_REPORT, "TIME_REPORT"),
        (at::COMMAND_NOT_FOUND, "COMMAND_NOT_FOUND"),
        (at::IDLE_TIMEOUT, "IDLE_TIMEOUT"),
        (at::PRE_RECORD, "PRE_RECORD"),
        (at::ON_EXIT, "ON_EXIT"),
        (at::ON_KEY, "ON_KEY"),
    ])
}

/// Whether `text` mentions `at::NAME` somewhere that is not the definition or this test's own kind
/// of listing.
///
/// A mention is enough. Pinning which function it was passed to would mean parsing Rust, and the
/// rule that matters — the *kind* of dispatch — is checked separately by the assertion in
/// `fire_or_defer` and `answer_hook_with`, which fires on every call rather than on the ones a
/// regex found.
fn mentions(text: &str, constant: &str) -> bool {
    text.match_indices(&format!("at::{constant}"))
        .any(|(offset, _)| {
            // `pub const PRE_CMD: usize = 0;` is the definition, not a use of it.
            let line_start = text[..offset].rfind('\n').map_or(0, |n| n + 1);
            !text[line_start..offset].contains("const ")
        })
}

/// Every hook in the table is fired by something in the shell.
///
/// The one that was not is what this file is for.
#[test]
fn every_hook_has_a_fire_site() {
    let names = constant_names();
    let files = sources();
    let mut dead = Vec::new();

    for (index, hook) in HOOKS.iter().enumerate() {
        let constant = names
            .get(&index)
            .unwrap_or_else(|| panic!("{} has no at:: constant", hook.name));
        // `lua/api/hooks.rs` is the table and the constants; a mention there proves nothing.
        let fired = files
            .iter()
            .any(|(path, text)| !path.ends_with("lua/api/hooks.rs") && mentions(text, constant));
        if !fired {
            dead.push(hook.name);
        }
    }

    assert!(
        dead.is_empty(),
        "these hooks can be attached to and nothing ever fires them, which is \
         indistinguishable from a misspelt hook name: {dead:?}"
    );
}

/// The dispatch a hook goes through matches what `Hook.answers` says about it.
///
/// Checked by which reach-back the fire site names. `answer_hook_with` and `ask_hook_here` read the
/// handler's return value; `fire_at_here` discards it and is free to defer the call to a moment
/// when the shell is idle. Sending an answering hook through the second one would silently drop a
/// `pre-cmd` veto — the command would run.
#[test]
fn answering_hooks_are_fired_by_the_dispatch_that_reads_an_answer() {
    let names = constant_names();
    let files = sources();

    for (index, hook) in HOOKS.iter().enumerate() {
        let constant = names[&index];
        for (path, text) in &files {
            if path.ends_with("lua/api/hooks.rs") || path.ends_with("lua/engine/hooks.rs") {
                continue;
            }
            for (offset, _) in text.match_indices(&format!("at::{constant}")) {
                // The dispatch is named just before the index, usually on the line above once
                // rustfmt has wrapped the call. 200 bytes covers the widest of them.
                let mut from = offset.saturating_sub(200);
                while from < offset && !text.is_char_boundary(from) {
                    from += 1;
                }
                let before = &text[from..offset];
                let Some(call) = [
                    "fire_at_here",
                    "answer_hook_with",
                    "ask_hook_here",
                    "fire_at",
                ]
                .iter()
                .filter_map(|name| before.rfind(name).map(|at| (at, *name)))
                .max_by_key(|(at, _)| *at)
                .map(|(_, name)| name) else {
                    continue;
                };
                let reads_the_answer = matches!(call, "answer_hook_with" | "ask_hook_here");
                assert_eq!(
                    reads_the_answer,
                    hook.answers,
                    "{} is fired by {call} in {}, which disagrees with HOOKS.answers = {}",
                    hook.name,
                    path.display(),
                    hook.answers
                );
            }
        }
    }
}
