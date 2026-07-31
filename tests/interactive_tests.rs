//! The line editor driven directly, with no pty (PLAN R10.1).
//!
//! Every rustyline trait `OsloHelper` implements is reachable from here except one: `validate`
//! takes a `ValidationContext`, whose constructor is `pub(crate)` in rustyline, so no test
//! outside that crate can build one. The verdict `validate` returns is
//! [`OsloHelper::input_status`] — exposed for exactly this reason — and that is what the
//! validator section below pins, together with the parser cross-check an integration test can
//! make and a unit test cannot: a buffer the editor calls `Complete` must be one the shell can
//! actually parse.

use oslo::env::Environment;

use oslo::interactive::{DEFAULT_PS2, InputStatus, OsloHelper, extract_current_word};
use oslo::parser::parse_bash_script;
use rustyline::Context;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::{History, MemHistory};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A helper over `env` with the dropdown off.
///
/// `Environment::new()` is not interactive, so `OsloHelper::new` already leaves the menu off and
/// the frecency table in memory; saying so explicitly keeps the tests honest if that default ever
/// changes, because with the menu on `complete` would block on the terminal.
fn helper(env: Environment) -> OsloHelper {
    let mut h = OsloHelper::new(Arc::new(Mutex::new(env)));
    h.set_menu(false);
    h
}

/// An environment whose `$PATH` is exactly `dir`, so command completion is deterministic.
///
/// Set *unexported*, and that is not a detail. Exporting reaches `unsafe { env::set_var }`
/// (`src/env/scope.rs`), which rewrites this test process's real `environ` from a libtest worker
/// thread while sixteen siblings in this binary are inside `Environment::new()`'s `env::vars()`
/// walk — the R10.3 data race, plus a wrecked `$PATH` for anything that spawns a command
/// afterwards. Nothing is lost: completion, hinting and highlighting all read `$PATH` through
/// `Environment::get_var`, never through `getenv`.
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

fn displays(h: &OsloHelper, line: &str, pos: usize) -> Vec<String> {
    let (_, cands) = h.candidates(line, pos);
    cands.into_iter().map(|c| c.display).collect()
}

/// Drop the SGR escapes the highlighter adds, leaving the text it was given.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Every escape this highlighter emits is `ESC [ … m`.
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ------------------------------------------------------- extract_current_word: quotes

#[test]
fn the_word_starts_at_an_open_quote_not_after_it() {
    // `wc -c "My Fi<TAB>` completes `My Fi`, and the replacement has to overwrite the quote too.
    assert_eq!(extract_current_word("wc -c \"My Fi", 12), (6, "\"My Fi"));
    assert_eq!(extract_current_word("wc -c 'My Fi", 12), (6, "'My Fi"));
}

#[test]
fn an_escaped_separator_stays_inside_the_word() {
    assert_eq!(extract_current_word(r"cat My\ Fi", 10), (4, r"My\ Fi"));
    // The line that used to wedge the prompt: the quote is escaped, so no quote is open.
    assert_eq!(extract_current_word(r"echo don\'t", 11), (5, r"don\'t"));
}

#[test]
fn a_separator_inside_quotes_does_not_end_the_word() {
    assert_eq!(extract_current_word("echo \"a | b", 11), (5, "\"a | b"));
    assert_eq!(extract_current_word("echo 'a; b", 10), (5, "'a; b"));
}

// ---------------------------------------------------- extract_current_word: operators

#[test]
fn every_operator_ends_the_word() {
    for (line, want) in [
        ("true && ec", (8, "ec")),
        ("true||ec", (6, "ec")),
        ("ls|gr", (3, "gr")),
        ("a;b", (2, "b")),
        ("(ec", (1, "ec")),
        ("echo hi >fi", (9, "fi")),
        ("sort <fi", (6, "fi")),
        ("echo `ec", (6, "ec")),
    ] {
        assert_eq!(extract_current_word(line, line.len()), want, "{line:?}");
    }
}

#[test]
fn only_the_text_left_of_the_cursor_is_the_word() {
    // The completer is asked wherever the cursor happens to be; the tail is not part of the word.
    assert_eq!(extract_current_word("cat hello world", 7), (4, "hel"));
    assert_eq!(extract_current_word("cat hello world", 15), (10, "world"));
}

#[test]
fn a_cursor_inside_a_multibyte_character_does_not_panic() {
    // A caller that computed the position from a grapheme count can land mid-character.
    let (start, text) = extract_current_word("cat héllo", 6);
    assert_eq!((start, text), (4, "h"));
}

// -------------------------------------------------------------- candidates for (line, pos)

#[test]
fn candidates_are_built_from_the_text_left_of_the_cursor() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file.txt"), b"x").unwrap();
    fs::write(dir.path().join("flag.txt"), b"x").unwrap();
    let h = helper(Environment::new());

    let line = format!("cat {}/fi extra-arg", dir.path().display());
    let pos = line.find(" extra").unwrap();
    let (start, cands) = h.candidates(&line, pos);

    assert_eq!(start, 4, "the replacement starts at the word, not the line");
    let names: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert_eq!(names, vec!["file.txt"], "{names:?}");
}

#[test]
fn a_dollar_word_completes_variable_names() {
    let mut env = Environment::new();
    env.set_var("ZZ_ALPHA", "1", false);
    env.set_var("ZZ_BETA", "2", false);
    let h = helper(env);

    let (start, cands) = h.candidates("echo $ZZ_", 9);
    assert_eq!(start, 5, "the `$` belongs to the word");
    let mut names: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["$ZZ_ALPHA", "$ZZ_BETA"]);
    assert_eq!(cands[0].kind.as_deref(), Some("variable"));
}

#[test]
fn a_completed_variable_keeps_its_dollar_unescaped() {
    let mut env = Environment::new();
    env.set_var("ZZ_ALPHA", "1", false);
    let h = helper(env);

    // `$` is on the escape list every other completion goes through, and escaping it here would
    // insert a word that expands to its own name. Only the unquoted case is asserted: inside an
    // open `"` the binding currently *does* escape it (completion.rs:56 does the opposite of the
    // comment above it), which is a live defect and not something a test should freeze.
    let (_, cands) = h.candidates("echo $ZZ_", 9);
    let reps: Vec<&str> = cands.iter().map(|c| c.replacement.as_str()).collect();
    assert_eq!(reps, vec!["$ZZ_ALPHA"], "{reps:?}");
}

#[test]
fn cd_offers_directories_only_and_marks_them_with_a_slash() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("adir")).unwrap();
    fs::write(dir.path().join("afile"), b"x").unwrap();
    let h = helper(Environment::new());

    let line = format!("cd {}/a", dir.path().display());
    let (_, cands) = h.candidates(&line, line.len());
    let names: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert_eq!(names, vec!["adir/"], "{names:?}");
    assert_eq!(cands[0].kind.as_deref(), Some("dir"));

    // Any other command sees both.
    let line = format!("cat {}/a", dir.path().display());
    assert_eq!(displays(&h, &line, line.len()).len(), 2);
}

#[test]
fn a_dotfile_is_offered_only_once_the_dot_is_typed() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".hidden"), b"x").unwrap();
    fs::write(dir.path().join("visible"), b"x").unwrap();
    let h = helper(Environment::new());

    let line = format!("cat {}/", dir.path().display());
    let names = displays(&h, &line, line.len());
    assert!(names.contains(&"visible".to_string()), "{names:?}");
    assert!(!names.contains(&".hidden".to_string()), "{names:?}");

    let line = format!("cat {}/.", dir.path().display());
    assert_eq!(displays(&h, &line, line.len()), vec![".hidden"]);
}

#[test]
fn nothing_matching_gives_an_empty_list_and_still_reports_the_word() {
    let h = helper(Environment::new());
    let line = "cat /nonexistent-directory-zz/f";
    let (start, cands) = h.candidates(line, line.len());
    assert_eq!(start, 4);
    assert!(cands.is_empty(), "{cands:?}");
}

#[test]
fn a_command_the_user_has_run_is_offered_first() {
    let dir = tempfile::tempdir().unwrap();
    // Same length, so only the ranking can separate them.
    make_exe(dir.path(), "zzaaa");
    make_exe(dir.path(), "zzbbb");
    let h = helper(env_with_path(dir.path()));

    assert_eq!(
        displays(&h, "zz", 2).first().map(String::as_str),
        Some("zzaaa")
    );
    h.record_command_use("zzbbb --flag");
    assert_eq!(
        displays(&h, "zz", 2).first().map(String::as_str),
        Some("zzbbb")
    );
}

// ------------------------------------------------------------------ hint selection

#[test]
fn a_line_already_in_the_history_is_hinted_from_it() {
    let dir = tempfile::tempdir().unwrap();
    let h = helper(env_with_path(dir.path()));
    let mut history = MemHistory::new();
    history.add("echo hello world").unwrap();
    let ctx = Context::new(&history);

    assert_eq!(h.hint("echo hel", 8, &ctx), Some("lo world".to_string()));
}

#[test]
fn history_wins_over_the_command_index() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "zzalpha");
    let h = helper(env_with_path(dir.path()));
    let mut history = MemHistory::new();
    history.add("zzbravo --flag").unwrap();
    let ctx = Context::new(&history);

    // A line the user has actually run beats any name we could rank: `zzalpha` is on `$PATH` and
    // is still not the answer.
    assert_eq!(h.hint("zz", 2, &ctx), Some("bravo --flag".to_string()));
}

#[test]
fn a_command_prefix_is_hinted_when_the_history_has_nothing() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "zzalpha");
    let h = helper(env_with_path(dir.path()));
    let history = MemHistory::new();
    let ctx = Context::new(&history);

    assert_eq!(h.hint("zzal", 4, &ctx), Some("pha".to_string()));
    // An argument is not a command name, so nothing is guessed for it.
    assert_eq!(h.hint("echo zzal", 9, &ctx), None);
}

#[test]
fn nothing_is_hinted_for_an_empty_line_or_from_the_middle_of_one() {
    let dir = tempfile::tempdir().unwrap();
    let h = helper(env_with_path(dir.path()));
    let mut history = MemHistory::new();
    history.add("echo hello world").unwrap();
    let ctx = Context::new(&history);

    assert_eq!(h.hint("", 0, &ctx), None);
    // The ghost text is drawn past the cursor; with the cursor mid-line it would overwrite what
    // is already there.
    assert_eq!(h.hint("echo hello world", 4, &ctx), None);
    // An exact match has no tail to suggest.
    assert_eq!(h.hint("echo hello world", 16, &ctx), None);
}

// ---------------------------------------------------------------- validator verdicts

#[test]
fn every_unfinished_construct_asks_for_another_line() {
    let h = helper(Environment::new());
    for line in [
        "for i in 1 2 3",
        "for i in 1 2 3; do",
        "while true; do",
        "until false; do",
        "if true",
        "if true; then",
        "case $x in",
        "f() {",
        "echo a &&",
        "echo a ||",
        "echo a |",
        "echo 'a",
        "echo \"a",
        "echo $(ls",
        "echo `ls",
        "cat <<EOF",
        "cat <<EOF\nbody",
        "cat <<-EOF\n\tbody",
    ] {
        assert_eq!(h.input_status(line), InputStatus::Incomplete, "{line:?}");
    }
}

#[test]
fn a_finished_line_is_run_rather_than_continued() {
    let h = helper(Environment::new());
    for line in [
        "",
        "   ",
        "echo hi",
        r"echo don\'t",
        "for i in 1 2 3; do echo $i; done",
        "cat <<EOF\nbody\nEOF",
        "if true; then echo y; fi",
    ] {
        assert_eq!(h.input_status(line), InputStatus::Complete, "{line:?}");
    }
}

#[test]
fn a_syntax_error_is_not_a_continuation() {
    let h = helper(Environment::new());
    // Another line cannot repair any of these, so the editor hands them to the executor, which
    // reports them the way a script would.
    for line in ["done", "fi", "esac", "echo hi )"] {
        assert_eq!(h.input_status(line), InputStatus::Invalid, "{line:?}");
    }
}

#[test]
fn the_editors_verdict_agrees_with_the_shells_parser() {
    let h = helper(Environment::new());
    // The point of the whole classifier: a buffer the editor accepts must be one the shell can
    // parse, or Enter runs something the user did not type.
    for line in [
        "echo hi",
        r"echo don\'t",
        "for i in 1 2 3; do echo $i; done",
        "cat <<EOF\nbody\nEOF",
        "echo 'a b' \"c d\"",
    ] {
        assert_eq!(h.input_status(line), InputStatus::Complete, "{line:?}");
        assert!(parse_bash_script(line).is_ok(), "{line:?}");
    }
    for line in ["done", "echo hi )"] {
        assert_eq!(h.input_status(line), InputStatus::Invalid, "{line:?}");
        assert!(parse_bash_script(line).is_err(), "{line:?}");
    }
}

#[test]
fn the_continuation_prompt_is_ps2_with_a_default() {
    let mut env = Environment::new();
    env.set_var("PS2", "…> ", false);
    assert_eq!(helper(env).continuation_prompt(), "…> ");
    assert_eq!(
        helper(Environment::new()).continuation_prompt(),
        DEFAULT_PS2
    );
}

// -------------------------------------------------------------------- highlighting

#[test]
fn colouring_never_changes_the_text_it_colours() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "zzreal");
    let h = helper(env_with_path(dir.path()));

    // The highlighter's output is what rustyline draws; if it does not spell the line back
    // exactly, the cursor lands in the wrong column.
    for line in [
        "zzreal -x",
        "echo \"a b\" $HOME | wc -l",
        "git commit -m 'x' && true",
        "echo don't",
        "cat <<EOF",
    ] {
        assert_eq!(strip_ansi(&h.highlight(line, line.len())), line, "{line:?}");
    }
}

/// An empty line has no syntax to paint, but it still gets the right prompt.
///
/// This used to assert `Cow::Borrowed("")`, which pinned a real bug: rustyline draws the prompt and
/// calls `highlight` with `""`, so returning the line untouched meant the right prompt appeared
/// only after the first keystroke. With no right prompt set there is still nothing to add.
#[test]
fn an_empty_line_carries_the_right_prompt_and_nothing_else() {
    let h = helper(Environment::new());
    assert_eq!(
        strip_ansi(&h.highlight("", 0)),
        "",
        "no right prompt, nothing added"
    );

    h.set_right_prompt(Some("RIGHT".to_string()), 4);
    let drawn = h.highlight("", 0);
    assert!(drawn.contains("RIGHT"), "{drawn:?}");
}

#[test]
fn a_ghost_hint_is_drawn_in_the_autosuggestion_colour() {
    let h = helper(Environment::new());

    // Colour 240 where the terminal can say it, which is the default.
    oslo::interactive::theme::set_depth(oslo::interactive::theme::Depth::Ansi256);
    assert_eq!(
        h.highlight_hint("lo world"),
        "\x1b[38;5;240mlo world\x1b[0m"
    );

    // On sixteen colours it degrades to whatever grey is nearest. Pinned rather than left
    // implicit, because naming an exact grey means accepting whatever the sixteen-slot palette
    // rounds it to.
    oslo::interactive::theme::set_depth(oslo::interactive::theme::Depth::Ansi16);
    assert_eq!(h.highlight_hint("lo world"), "\x1b[37mlo world\x1b[0m");
}

/// The lexer is the half that needs no shell, so it is the half an integration test can pin
/// exactly. Resolution — builtin vs command vs error — is unit-tested where the context can be
/// faked; here the point is that each span comes out with the right lexical role.
#[test]
fn the_lexer_gives_every_span_its_role() {
    use oslo::interactive::highlight::{Role, lex};

    let spans: Vec<(String, Role)> = lex("echo \"a b\" $HOME | wc -l")
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| (s.text, s.role))
        .collect();

    let want = [
        ("echo", Role::CommandWord),
        ("\"a b\"", Role::DoubleQuote),
        ("$HOME", Role::Variable),
        ("|", Role::Operator),
        // The word after a pipe is a command again, not an argument.
        ("wc", Role::CommandWord),
        ("-l", Role::Word),
    ];
    assert_eq!(spans.len(), want.len(), "{spans:?}");
    for (got, (text, role)) in spans.iter().zip(want) {
        assert_eq!((got.0.as_str(), got.1), (text, role));
    }
}

#[test]
fn an_unterminated_quote_colours_the_rest_of_the_line() {
    use oslo::interactive::highlight::{Role, lex};
    let spans = lex("echo \"a b");
    let last = spans.last().unwrap();
    assert_eq!(
        (last.text.as_str(), last.role),
        ("\"a b", Role::DoubleQuote)
    );
}
