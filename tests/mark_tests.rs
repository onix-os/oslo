//! `mark`, `@name`, and the leading position that belongs to neither.
//!
//! The `@` sigil names a directory you marked. The *start of a line* does not: a leading symbol is
//! reserved, so `@proj` typed on its own is a word the shell declines to read rather than a path it
//! quietly produces and then fails to execute.

mod common;

use std::path::Path;
use std::process::{Command, Stdio};

/// An interactive shell in `dir`, with its own data directory, running `script`.
///
/// Interactive because every one of these is: `@name` does not expand in a script, deliberately, so
/// a non-interactive run would prove nothing about any of it.
fn interactive(dir: &Path, data: &Path, script: &str) -> (String, String) {
    let output = Command::new(common::oslo_bin())
        .arg("-i")
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("XDG_DATA_HOME", data)
        .env("HOME", dir)
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A directory to stand in and a data directory to keep marks in.
fn somewhere(name: &str) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let root = tempfile::tempdir().expect("tempdir");
    let here = root.path().join(name);
    std::fs::create_dir_all(here.join("src")).expect("mkdir");
    let data = root.path().join("data");
    std::fs::create_dir_all(&data).expect("mkdir");
    (root, here, data)
}

/// **The case it exists for.** `mark` names this directory, `@name` reaches it from anywhere, and
/// `mark` again takes it back.
#[test]
fn mark_names_a_directory_and_unnames_it() {
    let (_root, here, data) = somewhere("proj");

    let (out, err) = interactive(
        &here,
        &data,
        "mark; echo @proj; echo @proj/src; mark; echo @proj",
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.first().copied(), Some("marked @proj"), "{out}{err}");
    assert_eq!(lines.get(1).copied(), Some(here.to_str().unwrap()), "{out}");
    assert_eq!(
        lines.get(2).copied(),
        Some(here.join("src").to_str().unwrap()),
        "the tail is kept: {out}"
    );
    assert_eq!(lines.get(3).copied(), Some("unmarked @proj"), "{out}");
    // Once it is gone the word is left exactly as it was typed, which is the safety rule the whole
    // shorthand rests on.
    assert_eq!(lines.get(4).copied(), Some("@proj"), "{out}");
}

/// **The start of a line is reserved.** A leading symbol is being kept for something else, so
/// `@name` must not expand there — it did, and the line then failed with `Is a directory`, which
/// is the position spoken for by an error nobody chose.
#[test]
fn a_leading_at_is_left_for_something_else() {
    let (_root, here, data) = somewhere("proj");

    let (_, _) = interactive(&here, &data, "mark");
    let (out, err) = interactive(&here, &data, "@proj; echo status=$?");
    assert!(
        err.contains("@proj: command not found"),
        "the word is declined, not expanded into a path: {err}"
    );
    assert!(!err.contains("Is a directory"), "{err}");
    // And no guess is offered about what it might have been: the shell decided not to read it.
    assert!(!err.contains("did you mean"), "{err}");
    assert!(out.contains("status=127"), "{out}{err}");
}

/// `=name` is untouched by that rule, and still names where a command lives in command position —
/// which is the whole of what that shorthand is for.
#[test]
fn the_other_shorthand_still_leads_a_line() {
    let (_root, here, data) = somewhere("proj");
    let (out, err) = interactive(&here, &data, "=echo ran");
    assert_eq!(out.trim(), "ran", "{out}{err}");
}

/// A mark outlives the shell that made it: it is a file, and the next shell reads it.
#[test]
fn a_mark_outlives_the_shell_that_made_it() {
    let (_root, here, data) = somewhere("proj");

    interactive(&here, &data, "mark");
    // A different shell, started fresh, standing somewhere else entirely.
    let (out, err) = interactive(Path::new("/"), &data, "echo @proj");
    assert_eq!(out.trim(), here.to_str().unwrap(), "{out}{err}");

    // And `mark -l` shows it.
    let (out, _) = interactive(Path::new("/"), &data, "mark -l");
    assert!(out.contains("@proj"), "{out}");
}

/// A name that already means somewhere else is refused rather than moved: walking into a second
/// `src` must not steal the first one's name.
#[test]
fn a_taken_name_is_refused() {
    let (_root, here, data) = somewhere("proj");
    let other = here.parent().expect("parent").join("proj2");
    std::fs::create_dir_all(&other).expect("mkdir");

    interactive(&here, &data, "mark one");
    let (_, err) = interactive(&other, &data, "mark one");
    assert!(err.contains("already"), "{err}");

    // The first mark is untouched.
    let (out, _) = interactive(Path::new("/"), &data, "echo @one");
    assert_eq!(out.trim(), here.to_str().unwrap(), "{out}");
}

/// **`@name` names a directory, so the glob after it is yours and has to run.** Substituted at the
/// end of expansion instead, `@proj/*.rs` reached the command with a literal `*` while `~/*.rs` and
/// `$M/*.rs` both expanded — and `echo "@proj"` expanded through its own quotes, because a finished
/// string no longer remembers it had any.
#[test]
fn a_mark_behaves_like_a_tilde() {
    let (_root, here, data) = somewhere("proj");
    std::fs::write(here.join("a.rs"), b"x").expect("write");
    std::fs::write(here.join("b.rs"), b"x").expect("write");
    interactive(&here, &data, "mark");

    let elsewhere = Path::new("/tmp");
    let (out, err) = interactive(elsewhere, &data, "echo @proj/*.rs");
    let listed = out.trim();
    assert_eq!(
        listed,
        format!(
            "{} {}",
            here.join("a.rs").display(),
            here.join("b.rs").display()
        ),
        "the glob after the mark must expand: {out}{err}"
    );

    // A quoted one is a literal, exactly as `echo "~"` prints a tilde.
    let (out, _) = interactive(elsewhere, &data, "echo \"@proj\"");
    assert_eq!(out.trim(), "@proj", "quotes turn it off");

    // And the plain forms still work.
    let (out, _) = interactive(elsewhere, &data, "echo @proj; echo @proj/a.rs");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.first().copied(), Some(here.to_str().unwrap()));
    assert_eq!(
        lines.get(1).copied(),
        Some(here.join("a.rs").to_str().unwrap())
    );
}

/// **The same test written two ways gives the same answer.**
///
/// `@name` was substituted at one call site — `expand_word_at`, arguments only — so `[ -d @proj ]`
/// was true while `[[ -d @proj ]]` was false, and `case @proj in /*)` did not match where
/// `case ~ in /*)` did. Two causes, one symptom: the other entry points never called the
/// substitution at all, and `[[ ]]` wraps each operand in a synthetic double quote that made the
/// word look like one the user had quoted deliberately.
#[test]
fn a_mark_expands_wherever_a_tilde_does() {
    let (_root, here, data) = somewhere("proj");
    interactive(&here, &data, "mark proj");

    let ask = |script: &str| {
        let (out, _) = interactive(here.parent().expect("parent"), &data, script);
        out.trim().to_string()
    };

    assert_eq!(ask("[ -d @proj ] && echo yes || echo no"), "yes");
    assert_eq!(
        ask("[[ -d @proj ]] && echo yes || echo no"),
        "yes",
        "`[[ ]]` must agree with `[ ]` about the same word"
    );
    assert_eq!(
        ask("case @proj in /*) echo yes;; *) echo no;; esac"),
        "yes",
        "`case` expands a tilde, so it expands a mark"
    );
}

/// And nowhere a tilde does not. A quoted `@name` is a literal, and a name standing for nothing
/// keeps its own text rather than becoming the filesystem root.
#[test]
fn a_quoted_or_unknown_mark_is_left_alone() {
    let (_root, here, data) = somewhere("proj");
    interactive(&here, &data, "mark proj");
    let ask = |script: &str| {
        let (out, _) = interactive(here.parent().expect("parent"), &data, script);
        out.trim().to_string()
    };

    assert_eq!(
        ask(r#"[[ -d "@proj" ]] && echo yes || echo no"#),
        "no",
        "a quoted mark is a literal, and widening the fix must not change that"
    );
    assert_eq!(ask("[[ -d @nosuchmark ]] && echo yes || echo no"), "no");
}
