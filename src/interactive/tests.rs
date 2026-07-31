//! Helper-level tests: the completer, hinter and validator driven directly, with no pty.
//!
//! Everything here goes through the public seams a test can reach — [`OsloHelper::candidates`],
//! [`OsloHelper::command_hint`], [`OsloHelper::input_status`] — plus `Completer::complete`
//! itself, which is callable once the dropdown is off. `Validator::validate` is *not* callable
//! from outside rustyline (`ValidationContext::new` is crate-private), which is why the
//! classifier is exposed separately.

use super::*;
use crate::env::Environment;
use rustyline::history::MemHistory;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn helper(env: Environment) -> OsloHelper {
    // `Environment::new()` is not interactive, so the helper neither draws a dropdown nor writes
    // a frecency file; `complete` therefore returns the whole candidate list.
    let mut h = OsloHelper::new(Arc::new(Mutex::new(env)));
    h.set_menu(false);
    h
}

/// Unexported deliberately: exporting reaches `unsafe { env::set_var }` (`crate::env::scope`),
/// which mutates the real `environ` of the test process from a libtest worker thread while other
/// tests in this binary walk it in `Environment::new()`. The readers under test
/// (`completion`, `hinting`, the highlighter) all go through `get_var`, so the flag buys nothing.
fn env_with_path(dir: &Path) -> Environment {
    let mut env = Environment::new();
    env.set_var("PATH", dir.to_str().unwrap(), false);
    env
}

fn make_exe(dir: &Path, name: &str) {
    let p = dir.join(name);
    fs::write(&p, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
}

fn replacements(h: &OsloHelper, line: &str) -> Vec<String> {
    let (_, cands) = h.candidates(line, line.len());
    cands.into_iter().map(|c| c.replacement).collect()
}

fn displays(h: &OsloHelper, line: &str) -> Vec<String> {
    let (_, cands) = h.candidates(line, line.len());
    cands.into_iter().map(|c| c.display).collect()
}

// ---------------------------------------------------------------- R9.2: quoting

#[test]
fn a_file_name_with_a_space_comes_back_escaped() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("My File.txt"), b"x").unwrap();
    let h = helper(Environment::new());

    let line = format!("wc -c {}/My", dir.path().display());
    let reps = replacements(&h, &line);
    assert_eq!(reps.len(), 1, "{reps:?}");
    // The bug: a bare `entry.file_name()` produced `wc -c My File.txt` and three wc errors.
    assert!(reps[0].ends_with("My\\ File.txt"), "{reps:?}");
    assert!(!reps[0].contains("My File.txt"), "{reps:?}");
}

#[test]
fn a_file_name_with_a_quote_or_a_glob_comes_back_escaped() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a*b"), b"x").unwrap();
    fs::write(dir.path().join("a'c"), b"x").unwrap();
    let h = helper(Environment::new());

    let line = format!("cat {}/a", dir.path().display());
    let mut reps = replacements(&h, &line);
    reps.sort();
    assert_eq!(reps.len(), 2, "{reps:?}");
    assert!(reps.iter().any(|r| r.ends_with("a\\*b")), "{reps:?}");
    assert!(reps.iter().any(|r| r.ends_with("a\\'c")), "{reps:?}");
}

#[test]
fn completing_inside_a_double_quote_stays_inside_it() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("My File.txt"), b"x").unwrap();
    let h = helper(Environment::new());

    let line = format!("wc -c \"{}/My", dir.path().display());
    let (start, cands) = h.candidates(&line, line.len());
    assert_eq!(
        &line[start..start + 1],
        "\"",
        "replacement must start at the quote"
    );
    assert_eq!(cands.len(), 1);
    assert!(cands[0].replacement.starts_with('"'));
    assert!(cands[0].replacement.ends_with("My File.txt\""));
}

#[test]
fn completing_inside_a_single_quote_stays_inside_it() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("My File.txt"), b"x").unwrap();
    let h = helper(Environment::new());

    let line = format!("wc -c '{}/My", dir.path().display());
    let cands = replacements(&h, &line);
    assert_eq!(cands.len(), 1);
    assert!(cands[0].starts_with('\''), "{cands:?}");
    assert!(cands[0].ends_with("My File.txt'"), "{cands:?}");
}

#[test]
fn a_partly_escaped_word_matches_the_file_it_names() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("My File.txt"), b"x").unwrap();
    let h = helper(Environment::new());

    // The user already escaped the space; the stem is `My File.`, not `My\ File.`.
    let line = format!("wc -c {}/My\\ File.", dir.path().display());
    let reps = replacements(&h, &line);
    assert_eq!(reps.len(), 1, "{reps:?}");
    assert!(reps[0].ends_with("My\\ File.txt"), "{reps:?}");
}

// ------------------------------------------------- R9.3: completion after an operator

#[test]
fn commands_complete_after_every_separator() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "ecfoo");
    let h = helper(env_with_path(dir.path()));

    for line in [
        "ec",
        "true && ec",
        "true || ec",
        "true; ec",
        "ls | ec",
        "(ec",
        "if true; then ec",
    ] {
        let names = displays(&h, line);
        // The old gate was `start == 0 || line[..start].trim().is_empty()`, so every one of these
        // but the first produced a bare BEL.
        assert!(
            names.contains(&"ecfoo".to_string()),
            "{line:?} gave {names:?}"
        );
        assert!(
            names.contains(&"echo".to_string()),
            "{line:?} gave {names:?}"
        );
    }
}

#[test]
fn a_redirection_target_completes_files_not_commands() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "ecfoo");
    fs::write(dir.path().join("ecdata"), b"x").unwrap();
    let h = helper(env_with_path(dir.path()));

    let line = format!("echo hi > {}/ec", dir.path().display());
    let names = displays(&h, &line);
    assert!(names.contains(&"ecdata".to_string()), "{names:?}");
    assert!(!names.contains(&"echo".to_string()), "{names:?}");
}

#[test]
fn an_operator_inside_quotes_is_not_a_command_boundary() {
    let h = helper(Environment::new());
    // `"true && ec` is one quoted argument; nothing in it starts a command.
    let (_, cands) = h.candidates("echo \"true && ec", 16);
    assert!(
        !cands.iter().any(|c| c.display == "echo"),
        "{:?}",
        cands.iter().map(|c| &c.display).collect::<Vec<_>>()
    );
}

#[test]
fn complete_returns_the_whole_candidate_list_without_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "zzalpha");
    make_exe(dir.path(), "zzbeta");
    let h = helper(env_with_path(dir.path()));

    let history = MemHistory::new();
    let ctx = Context::new(&history);
    let (start, pairs) = h.complete("true && zz", 10, &ctx).unwrap();
    assert_eq!(start, 8);
    let mut names: Vec<&str> = pairs.iter().map(|p| p.display.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["zzalpha", "zzbeta"]);
}

// ------------------------------------------------------------ R9.1: multi-line input

#[test]
fn the_validator_agrees_with_the_classifier() {
    let h = helper(Environment::new());
    assert_eq!(h.input_status("echo hi"), InputStatus::Complete);
    assert_eq!(h.input_status(r"echo don\'t"), InputStatus::Complete);
    assert_eq!(h.input_status("for i in 1 2 3"), InputStatus::Incomplete);
    assert_eq!(h.input_status("cat <<EOF"), InputStatus::Incomplete);
    assert_eq!(
        h.input_status("cat <<EOF\nbody\nEOF"),
        InputStatus::Complete
    );
    assert_eq!(h.input_status("done"), InputStatus::Invalid);
}

#[test]
fn the_continuation_prompt_honours_ps2() {
    let mut env = Environment::new();
    env.set_var("PS2", "…> ", false);
    let h = helper(env);
    assert_eq!(h.continuation_prompt(), "…> ");
    assert_eq!(
        helper(Environment::new()).continuation_prompt(),
        DEFAULT_PS2
    );
}

// ------------------------------------------------------------------- R9.6: frecency

#[test]
fn a_word_that_already_names_a_command_is_not_hinted_past() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "exitsnoop-bpfcc");
    let h = helper(env_with_path(dir.path()));

    // `exit` is a builtin, so it *is* the answer; suggesting `exitsnoop-bpfcc` was the bug.
    assert_eq!(h.command_hint("exit", 4), None);
    // A genuine prefix still gets a suggestion, and the builtin beats the stranger.
    assert_eq!(h.command_hint("exi", 3), Some("t".to_string()));
}

#[test]
fn hints_are_ranked_by_use() {
    let dir = tempfile::tempdir().unwrap();
    // Same length, so nothing but the score and the alphabet can separate them.
    make_exe(dir.path(), "zzaaa");
    make_exe(dir.path(), "zzbbb");
    let h = helper(env_with_path(dir.path()));

    // Alphabetical order alone always answers `zzaaa` — that is what "the tracker has no
    // callers" looked like from the prompt.
    assert_eq!(h.command_hint("zz", 2), Some("aaa".to_string()));
    h.record_command_use("zzbbb --flag");
    assert_eq!(h.command_hint("zz", 2), Some("bbb".to_string()));
}

#[test]
fn hints_prefer_shell_names_over_strangers() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "cdrecord");
    let mut env = env_with_path(dir.path());
    env.set_alias("cdx", "cd /x");
    let h = helper(env);

    // `cd` is a builtin and `cdx` an alias; both beat `cdrecord`, and `cdx` is shorter.
    assert_eq!(h.command_hint("cdr", 3), Some("ecord".to_string()));
    assert_eq!(h.command_hint("cd", 2), None, "cd already names a builtin");
}

#[test]
fn hints_do_not_fire_on_arguments_or_inside_quotes() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "zzalpha");
    let h = helper(env_with_path(dir.path()));
    assert_eq!(h.command_hint("echo zz", 7), None);
    assert_eq!(h.command_hint("\"zz", 3), None);
}

#[test]
fn an_accepted_completion_is_counted() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "zzsingle");
    let h = helper(env_with_path(dir.path()));

    assert_eq!(h.frecency_score("zzsingle"), 0.0);
    let history = MemHistory::new();
    let ctx = Context::new(&history);
    // Exactly one candidate, so rustyline inserts it: that is an acceptance.
    let (_, pairs) = h.complete("zzsing", 6, &ctx).unwrap();
    assert_eq!(pairs.len(), 1);
    assert!(h.frecency_score("zzsingle") > 0.0);
}

#[test]
fn a_completed_file_name_does_not_enter_the_command_ranking() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("solo.txt"), b"x").unwrap();
    let h = helper(Environment::new());

    let history = MemHistory::new();
    let ctx = Context::new(&history);
    let line = format!("cat {}/solo", dir.path().display());
    let (_, pairs) = h.complete(&line, line.len(), &ctx).unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(h.frecency_score("solo.txt"), 0.0);
}

#[test]
fn every_command_in_an_accepted_line_is_counted() {
    let h = helper(Environment::new());
    h.record_command_use("ls -l | grep x && echo done");
    for name in ["ls", "grep", "echo"] {
        assert!(h.frecency_score(name) > 0.0, "{name}");
    }
    assert_eq!(h.frecency_score("-l"), 0.0);
    assert_eq!(h.frecency_score("x"), 0.0);
}

// ------------------------------------------------------------------ R9.5: keystroke cost

#[test]
fn a_warm_keystroke_does_not_walk_path() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..200 {
        make_exe(dir.path(), &format!("bench{}", i));
    }
    let h = helper(env_with_path(dir.path()));

    // Warm the index, then time a hint the way the prompt would issue it.
    let _ = h.command_hint("ben", 3);
    let start = std::time::Instant::now();
    for _ in 0..20 {
        let _ = h.command_hint("ben", 3);
    }
    let per_keystroke = start.elapsed() / 20;
    assert!(
        per_keystroke < std::time::Duration::from_millis(1),
        "warm keystroke cost {per_keystroke:?}, budget 1ms"
    );
}

/// A command that resolves and one that does not must not be painted the same, whatever the
/// theme says they should be painted *as*. Asserting the escape itself would pin the default
/// theme rather than the behaviour.
#[test]
fn highlighting_tells_a_real_command_from_an_unknown_one() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "zzreal");
    let h = helper(env_with_path(dir.path()));
    crate::interactive::theme::set_depth(crate::interactive::theme::Depth::Ansi256);

    let theme = crate::interactive::theme::current();
    let depth = crate::interactive::theme::depth();
    let command = theme.syntax.command.open(depth);
    let error = theme.syntax.error.open(depth);
    let builtin = theme.syntax.builtin.open(depth);
    assert_ne!(command, error, "a real and a missing command look the same");

    assert!(
        h.highlight("zzreal -x", 9)
            .contains(&format!("{command}zzreal")),
        "{:?}",
        h.highlight("zzreal -x", 9)
    );
    assert!(
        h.highlight("zzfake -x", 9)
            .contains(&format!("{error}zzfake")),
        "{:?}",
        h.highlight("zzfake -x", 9)
    );
    // Builtins resolve without any file existing, and take their own colour.
    assert!(
        h.highlight("cd /tmp", 7).contains(&format!("{builtin}cd")),
        "{:?}",
        h.highlight("cd /tmp", 7)
    );
}

// ------------------------------------------------------------- spec-driven completion

#[test]
fn subcommands_come_from_the_spec_for_this_command_not_the_first_one() {
    let h = helper(Environment::new());
    // `ls |` starts a new command, so the spec looked up must be `git`, not `ls`.
    let names = displays(&h, "ls | git comm");
    assert!(names.contains(&"commit".to_string()), "{names:?}");

    let flags = displays(&h, "git commit --a");
    assert!(flags.iter().any(|f| f.starts_with("--a")), "{flags:?}");
}

#[test]
fn a_name_that_is_both_a_builtin_and_a_file_is_offered_once() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "echo");
    let h = helper(env_with_path(dir.path()));

    let names = displays(&h, "ech");
    assert_eq!(
        names.iter().filter(|n| *n == "echo").count(),
        1,
        "{names:?}"
    );
}

// ------------------------------------------------------------- path suggestions

/// fish's third autosuggestion source: the argument, which neither history nor the command index
/// can answer for.
#[test]
fn a_path_argument_is_suggested_from_the_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
    std::fs::create_dir(dir.path().join("build")).unwrap();

    let h = helper(Environment::new());
    let base = dir.path().display().to_string();

    let line = format!("cat {base}/not");
    assert_eq!(
        h.path_hint(&line, line.len()).as_deref(),
        Some("es.txt"),
        "no suggestion for {line}"
    );

    // A directory is suggested with its trailing slash, so the next keystroke continues into it.
    let line = format!("ls {base}/bui");
    assert_eq!(h.path_hint(&line, line.len()).as_deref(), Some("ld/"));
}

/// A bare word at the start of a line is a command to look up, not a file in the working
/// directory — suggesting `./notes.txt` for `no` would be nonsense.
///
/// Absolute paths throughout: `set_current_dir` is process-wide, and this binary runs its tests on
/// sixteen threads at once, so changing the working directory here would move it under every
/// other test that happened to be running.
#[test]
fn a_command_word_is_never_suggested_as_a_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
    let base = dir.path().display().to_string();
    let h = helper(Environment::new());

    // In command position, even a stem that names a real file suggests nothing as a path.
    let line = format!("{base}/not");
    assert_eq!(h.path_hint(&line, line.len()), None);

    // The same stem as an argument is fair game.
    let line = format!("cat {base}/not");
    assert_eq!(h.path_hint(&line, line.len()).as_deref(), Some("es.txt"));
}

/// Every argument would otherwise suggest `.git`, which is never what was meant.
#[test]
fn a_dotfile_is_only_suggested_once_the_dot_is_typed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".hidden"), b"x").unwrap();
    let base = dir.path().display().to_string();
    let h = helper(Environment::new());

    let line = format!("cat {base}/");
    assert_eq!(h.path_hint(&line, line.len()), None);

    let line = format!("cat {base}/.hid");
    assert_eq!(h.path_hint(&line, line.len()).as_deref(), Some("den"));
}
