//! **A new diagnostic that names a word gets a caret, or says why not.**
//!
//! The two other suites hold the *behaviour*: `diagnostics_stay_plain.rs` that a pipe sees what it
//! always saw, `diagnostics_draw_a_caret.rs` that a terminal sees the report. Neither notices the
//! thing most likely to happen next — a builtin written six months from now, printing
//! `mycmd: {operand}: invalid whatever` with an `eprintln!` because that is what the file beside it
//! used to do.
//!
//! There is nothing to see when that happens. The message is perfectly ordinary; it is only wrong
//! next to the report the message beside it draws. So this scans the source instead.
//!
//! # What counts as pointing at a word
//!
//! `{}name: {word}: reason` — an origin, a command, **a placeholder**, and a reason. That shape is
//! a diagnostic *about an operand*, which is exactly the population a caret is for. A message with
//! no placeholder in the middle (`cd: HOME not set`) is about a condition rather than a word and is
//! not scanned at all.
//!
//! The same shape `tests/builtin_diagnostics_tests.rs` uses to find hardcoded prefixes, for the
//! same reason: a rule nothing checks is a rule that lasts until the next contributor.

use std::path::{Path, PathBuf};

/// Where a diagnostic about an operand may be written.
const SPEAKING_ABOUT_A_WORD: [&str; 2] = [
    "crates/oslo-shell/src/env/builtins",
    "crates/oslo-shell/src/data",
];

/// The ones still printing a bare `eprintln!`, and why each one is not converted.
///
/// **A reason per entry, not a list.** "Not done yet" is not a reason; every line below says what
/// about that site makes a caret unavailable or unhelpful, and a site that gains what it lacks
/// should be converted and the row removed.
const KEPT: &[(&str, &str)] = &[
    (
        "printf: {}: invalid number",
        "`Spec::render` is handed one argument and no argv — the words are three frames up, and \
         threading them through a formatter called once per conversion is a lot of signature for \
         a caret under the only word in the message",
    ),
    (
        "printf: {value}: invalid number",
        "the same function, the same reason",
    ),
    (
        "read: {}: {err}",
        "an I/O failure on a descriptor: the word is the *name* being read into, which is not what \
         went wrong",
    ),
    (
        "nav: {program}: {error}",
        "the program is one oslo chose to run, not one the person typed — a caret under it would \
         point at oslo's own decision",
    ),
    (
        "wait: {}: no such job",
        "`resolve` takes an id and no argv; worth converting when `wait` next has a reason to \
         touch its option parsing",
    ),
    (
        "declare: {}: not found",
        "`print_variables` takes `names` and no argv — the same shape as `export`'s, and the same \
         small threading job",
    ),
    (
        "command: {}: not found",
        "`describe` takes a name and no argv",
    ),
    (
        "export: {}: not a function",
        "`export_functions` takes `names` and no argv",
    ),
    (
        "export: {}: readonly variable",
        "`unexport` takes one name and no argv",
    ),
    (
        "ulimit: {}: invalid number",
        "`set_one` takes a flag and an operand and no argv",
    ),
];

/// Every `.rs` file under the scanned directories, except the test modules beside them.
fn sources() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    let mut stack: Vec<PathBuf> = SPEAKING_ABOUT_A_WORD.iter().map(|d| root.join(d)).collect();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.file_name().is_some_and(|n| n != "tests.rs")
            {
                let text =
                    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
                found.push((path, text));
            }
        }
    }
    assert!(found.len() > 40, "the scan found almost nothing: {found:?}");
    found
}

/// The `{}name: {word}: reason` format strings a file passes to `eprintln!`.
fn operand_diagnostics(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in text.match_indices("eprintln!(") {
        let rest = &text[at..];
        // The first string literal after the macro name, which is the format string.
        let Some(open) = rest.find('"') else { continue };
        let Some(close) = rest[open + 1..].find('"') else {
            continue;
        };
        let format = &rest[open + 1..open + 1 + close];
        let Some(body) = format.strip_prefix("{}") else {
            continue;
        };
        // `name: {something}: reason` — a command, a placeholder, and a reason for it.
        let mut parts = body.splitn(3, ": ");
        let (Some(name), Some(word), Some(reason)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let named = name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == ' ' || c == '-');
        if named && word.starts_with('{') && word.ends_with('}') && !reason.is_empty() {
            found.push(body.to_string());
        }
    }
    found
}

/// The rule.
#[test]
fn a_diagnostic_about_a_word_draws_a_caret_or_says_why_not() {
    let mut unexplained: Vec<String> = Vec::new();
    for (path, text) in sources() {
        for message in operand_diagnostics(&text) {
            if KEPT.iter().any(|(kept, _)| *kept == message) {
                continue;
            }
            unexplained.push(format!("{}: {message}", path.display()));
        }
    }
    assert!(
        unexplained.is_empty(),
        "these print a bare `eprintln!` about a word somebody typed.\n\
         Give each one `crate::env::complain(...)`, or add it to KEPT with a reason:\n  {}",
        unexplained.join("\n  ")
    );
}

/// **`KEPT` may not rot.** An entry for a message nobody prints any more is a reason that reads as
/// current and is not, which is worse than no entry — and the way it happens is a site being
/// converted without its row being taken out.
#[test]
fn every_kept_entry_names_a_message_that_exists() {
    let sources = sources();
    for (message, reason) in KEPT {
        assert!(
            !reason.trim().is_empty(),
            "`{message}` is kept with no reason"
        );
        let found = sources
            .iter()
            .any(|(_, text)| text.contains(&format!("\"{{}}{message}\"")));
        assert!(
            found,
            "KEPT names `{message}`, which nothing prints any more"
        );
    }
}

/// The scanner has to see the shape it is looking for, and not see the ones it is not — otherwise
/// it passes by finding nothing, which is the failure mode of every source-scanning test.
#[test]
fn the_scanner_recognises_the_shape() {
    let found = operand_diagnostics(
        r#"
        eprintln!("{}kill: {spec}: invalid signal specification", origin_now());
        eprintln!("{}cd: HOME not set", origin_now());
        eprintln!("{}ui input: {other}: unknown option", origin_now());
        eprintln!("{USAGE}");
        println!("{}not: {a}: an eprintln", x);
        "#,
    );
    assert_eq!(
        found,
        vec![
            "kill: {spec}: invalid signal specification".to_string(),
            "ui input: {other}: unknown option".to_string(),
        ],
        "a condition, a usage block and a println are not diagnostics about a word"
    );
}
