//! No name that can carry structure may be a command somebody already has.
//!
//! The POSIX guarantee rests on a claim `data::tools::register_all` makes about itself: *every name
//! here is one oslo invented, so a script written before oslo existed cannot name any of them.*
//! `tests/posix_stays_on_the_byte_path.rs` checks the consequence across 416 corpus scripts; this
//! checks the premise, which is the stronger statement — a corpus can only fail on a line somebody
//! thought to write down.
//!
//! # It has already been wrong once
//!
//! `uniq` was registered as a row verb. Alone it ran coreutils, and after a *bytes* producer it ran
//! coreutils — so every test passed and the corpus was clean. But after a **rows** producer it was
//! the verb, and `ls | uniq` printed ls's columns instead of deduplicating its lines. It is
//! `distinct` now, and this test is what would have caught it.
//!
//! An entry in [`DELIBERATE`] is not an exemption; it is a claim that the collision was chosen and
//! is understood. Adding one means saying why in the table.

use std::process::Command;

/// Every name that can carry structure, as `register_all` declares them.
///
/// Written out rather than read from the shell, so this test fails when somebody *adds* a name and
/// has to come and think about it here.
const STRUCTURED: &[&str] = &[
    "df", "ps", "ls", "where", "lines", "parse", "from", "cols", "get", "sort-by", "first", "last",
    "length", "each", "group-by", "count", "distinct", "stats", "to",
];

/// Names that collide with a real command on purpose, and why that is safe.
const DELIBERATE: &[(&str, &str)] = &[
    (
        "df",
        "a producer: oslo's own `df` replaces coreutils' deliberately, and prints the same shape",
    ),
    (
        "ps",
        "a producer, the same way — and it stays coreutils when it is the only command in a line",
    ),
    (
        "ls",
        "a producer, and the one people notice: it is coreutils unless a rows consumer follows it",
    ),
    (
        "last",
        "util-linux's `last` shows logins. It collides only in `<rows producer> | last`, which is \
         not an idiom anybody writes — but it is a collision, and it is recorded rather than \
         discovered later",
    ),
];

/// The utilities a script may reasonably expect to find. POSIX.1 plus what coreutils adds.
const REAL_COMMANDS: &[&str] = &[
    "awk", "basename", "cat", "chmod", "chown", "cmp", "comm", "cp", "cut", "date", "dd", "df",
    "diff", "dirname", "du", "echo", "env", "expr", "file", "find", "fold", "grep", "head", "id",
    "join", "kill", "ln", "ls", "mkdir", "mv", "nl", "od", "paste", "pr", "printf", "ps", "pwd",
    "rm", "rmdir", "sed", "seq", "sh", "sleep", "sort", "split", "stat", "tail", "tee", "test",
    "touch", "tr", "true", "tsort", "uname", "uniq", "wc", "who", "xargs", "yes",
    // Not POSIX, but on nearly every Linux box and just as likely to be piped into.
    "last", "column", "jq", "tac", "shuf", "nproc", "realpath", "timeout", "watch",
];

#[test]
fn no_structured_name_is_a_command_somebody_already_has() {
    let deliberate: Vec<&str> = DELIBERATE.iter().map(|(name, _)| *name).collect();
    let mut clashes = Vec::new();
    for name in STRUCTURED {
        if deliberate.contains(name) {
            continue;
        }
        if REAL_COMMANDS.contains(name) {
            clashes.push(*name);
        }
    }
    assert!(
        clashes.is_empty(),
        "these carry structure and are also commands a script may already call: {clashes:?}.\n\
         A rows producer piped into one of them silently changes what the line means — see this \
         file's notes on `uniq`. Rename the verb, or add it to DELIBERATE with the reason."
    );
}

/// The same question asked of the machine this is running on, which catches a name that is not in
/// the list above but is installed here anyway.
///
/// A warning rather than a failure: what is installed varies by machine and by container, and a
/// test that depends on that is a test that fails for the wrong reason.
#[test]
fn report_any_structured_name_installed_on_this_machine() {
    let deliberate: Vec<&str> = DELIBERATE.iter().map(|(name, _)| *name).collect();
    let mut found = Vec::new();
    for name in STRUCTURED {
        if deliberate.contains(name) || REAL_COMMANDS.contains(name) {
            continue;
        }
        let installed = Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {name}"))
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        if installed {
            found.push(*name);
        }
    }
    if !found.is_empty() {
        eprintln!(
            "note: these structured names are installed as commands here and are not recorded \
             anywhere: {found:?}"
        );
    }
}

/// Every entry has to say why, or it is an exemption pretending to be a decision.
#[test]
fn every_deliberate_collision_gives_a_reason() {
    for (name, because) in DELIBERATE {
        assert!(
            because.len() > 30,
            "{name} is allowed to collide without saying why"
        );
        assert!(
            STRUCTURED.contains(name),
            "{name} is recorded as a collision but carries no structure"
        );
    }
}
