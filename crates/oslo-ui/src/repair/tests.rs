use super::*;

/// A `$PATH` with a few real-looking names in it, and nothing else.
fn path() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["lsblk", "cargo", "git", "systemctl"] {
        let file = dir.path().join(name);
        std::fs::write(&file, b"#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
    }
    let text = dir.path().to_string_lossy().to_string();
    (dir, text)
}

const NO_HAS: &dyn Fn(&str) -> bool = &|_: &str| false;
const NO_BEGINS: &dyn Fn(&str) -> bool = &|_: &str| false;
const NO_ALL: &dyn Fn() -> Vec<String> = &|| Vec::new();

/// A shell with no builtins, aliases or functions of its own — `$PATH` is the whole story.
fn nothing_is_known() -> Known<'static> {
    Known {
        has: NO_HAS,
        begins: NO_BEGINS,
        all: NO_ALL,
    }
}

/// **The case this exists for.** A misspelled command is respelled from `$PATH` alone — no history,
/// no model, nothing learned.
#[test]
fn a_misspelled_command_is_respelled_from_the_path() {
    let (_dir, path) = path();
    assert_eq!(
        of("lsvlk", &path, &nothing_is_known()).as_deref(),
        Some("lsblk")
    );
    // The rest of the line is left exactly as typed.
    assert_eq!(
        of("lsvlk -f /dev/sda", &path, &nothing_is_known()).as_deref(),
        Some("lsblk -f /dev/sda")
    );
}

/// A command that exists is not a mistake, however unusual it looks.
#[test]
fn a_real_command_is_left_alone() {
    let (_dir, path) = path();
    assert!(of("lsblk", &path, &nothing_is_known()).is_none());
    assert!(of("git status", &path, &nothing_is_known()).is_none());
}

/// **A half-typed command is not a wrong one**, which is both the correct answer and the one that
/// keeps the edit distance off the typing path — see the note in [`super::spelling`].
#[test]
fn a_word_that_starts_a_real_command_is_unfinished() {
    let (_dir, path) = path();
    for typed in ["c", "ca", "car", "carg", "g", "sys"] {
        assert!(
            of(typed, &path, &nothing_is_known()).is_none(),
            "{typed:?} is on the way to something, not a mistake"
        );
    }
    // And the moment it stops being a prefix of anything, it is a mistake again.
    assert_eq!(
        of("crgo", &path, &nothing_is_known()).as_deref(),
        Some("cargo")
    );
}

/// A shell whose own names are `names`, for the tests that need some.
macro_rules! shell {
    ($known:ident, $($name:literal),+ $(,)?) => {
        let names: Vec<String> = vec![$($name.to_string()),+];
        let has = |n: &str| names.iter().any(|x| x == n);
        let begins = |s: &str| names.iter().any(|x| x.starts_with(s));
        let all = || names.clone();
        let $known = Known { has: &has, begins: &begins, all: &all };
    };
}

/// The names only the shell knows are commands too, and must not be respelled into something else.
#[test]
fn a_builtin_or_alias_is_a_real_command() {
    let (_dir, path) = path();
    shell!(known, "gti");
    assert!(
        of("gti push", &path, &known).is_none(),
        "an alias someone deliberately named `gti` is not a typo for `git`"
    );
}

/// **A half-typed builtin is unfinished too.** `$PATH` has never heard of `source`, so `sourc` was
/// a word beginning nothing at all and got "corrected" into an unrelated program.
#[test]
fn a_word_that_starts_a_shell_name_is_unfinished() {
    let (_dir, path) = path();
    shell!(known, "source", "unalias", "readonly");
    for typed in ["s", "so", "sour", "sourc", "una", "read"] {
        assert!(
            of(typed, &path, &known).is_none(),
            "{typed:?} is on the way to a name the shell has, not a mistake"
        );
    }
}

/// **A misspelled builtin is respelled to the builtin**, not to whatever `$PATH` had nearest.
#[test]
fn a_misspelled_shell_name_is_respelled_to_it() {
    let (_dir, path) = path();
    shell!(known, "source", "unalias", "readonly");
    // A transposition, which is what people actually type.
    assert_eq!(
        of("soruce x", &path, &known).as_deref(),
        Some("source x"),
        "the builtin is nearer than anything on $PATH"
    );
    // And an ordinary edit.
    assert_eq!(of("unalis x", &path, &known).as_deref(), Some("unalias x"));
}

/// A path or a variable is not a name to spell-check.
#[test]
fn only_a_bare_command_word_is_checked() {
    let (_dir, path) = path();
    assert!(of("./lsvlk", &path, &nothing_is_known()).is_none());
    assert!(of("/usr/bin/lsvlk", &path, &nothing_is_known()).is_none());
    assert!(of("$editor", &path, &nothing_is_known()).is_none());
}

/// **An assignment prefix is not the command.** `FOO=1 lsvlk` runs `lsvlk`, and reading only the
/// first word left the correction silent about a line the highlighter was already painting as a
/// command that does not exist — two layers disagreeing about which word is the command.
#[test]
fn an_assignment_prefix_is_stepped_over() {
    let (_dir, path) = path();
    assert_eq!(
        of("FOO=1 lsvlk", &path, &nothing_is_known()).as_deref(),
        Some("FOO=1 lsblk"),
        "the prefix is kept and the command is corrected"
    );
    // However many there are.
    assert_eq!(
        of("FOO=1 BAR=2 lsvlk -f", &path, &nothing_is_known()).as_deref(),
        Some("FOO=1 BAR=2 lsblk -f")
    );
    // And a line that is only assignments names no command at all.
    assert!(of("FOO=1", &path, &nothing_is_known()).is_none());
    assert!(of("FOO=1 BAR=2", &path, &nothing_is_known()).is_none());
    // A word that merely contains `=` is not an assignment, so it is not stepped over either.
    assert!(of("1FOO=1 lsvlk", &path, &nothing_is_known()).is_none());
}

/// Nothing typed, nothing to say.
#[test]
fn an_empty_line_is_not_a_mistake() {
    let (_dir, path) = path();
    assert!(of("", &path, &nothing_is_known()).is_none());
    assert!(of("   ", &path, &nothing_is_known()).is_none());
}

/// **The gate on the model.** A retyping is offered; a different command is not, however well the
/// model thinks of it. Without this the hint would be drawn under every correct line ever typed.
#[test]
fn only_a_plausible_retyping_is_offered() {
    assert!(plausible("cargo buidl", "cargo build"));
    assert!(plausible("git stauts", "git status"));
    assert!(plausible("lsvlk", "lsblk"));

    assert!(
        !plausible("git status", "git status"),
        "not a change at all"
    );
    assert!(
        !plausible("git status", "git push"),
        "a different command is a prediction, not a repair"
    );
    assert!(
        !plausible("cargo build", "cargo build --release"),
        "adding a flag is not fixing a typo"
    );
    assert!(
        !plausible("ls", "systemctl"),
        "nothing in common is nothing to do with it"
    );
}

/// The budget grows with the line, but slowly.
#[test]
fn a_longer_line_may_differ_by_more() {
    // One edit is allowed at any length.
    assert!(plausible("ls -l", "ls -h"));
    // Two edits in a long line is still a typo; a third word is not.
    assert!(plausible(
        "systemctl restart netwrking",
        "systemctl restart networking"
    ));
    assert!(!plausible(
        "systemctl restart networking",
        "systemctl restart bluetoothd"
    ));
}

/// **Only the words that changed are bracketed**, and the arrow and the rest stay ghost.
///
/// Asserted on the plain text with styling off, so it is about *what is marked* rather than about
/// which escape says so.
#[test]
fn only_the_changed_words_are_bracketed() {
    let plain = Style::default();
    let drawn = |typed, fixed| annotate(typed, fixed, &plain, &plain, Depth::None);

    assert_eq!(drawn("lsvlk", "lsblk"), "-> [lsblk]");
    assert_eq!(
        drawn("systemclt status", "systemctl status"),
        "-> [systemctl] status",
        "the word that was already right is not marked"
    );
    assert_eq!(
        drawn("echo hello wrold", "echo hello world"),
        "-> echo hello [world]"
    );
}

/// A correction longer than what was typed marks the words it added.
#[test]
fn a_word_with_nothing_to_compare_to_is_a_change() {
    let plain = Style::default();
    assert_eq!(
        annotate("ls -l", "ls -l -a", &plain, &plain, Depth::None),
        "-> ls -l [-a]"
    );
}

/// The two styles are one colour: the ghost, and the ghost reversed.
#[test]
fn the_correction_is_the_ghost_inverted() {
    let ghost = crate::theme::Syntax::default().autosuggestion;
    let repair = crate::theme::Syntax::default().repair;
    assert_eq!(repair.fg, ghost.fg, "the same colour");
    assert!(repair.reverse && !ghost.reverse, "turned inside out");
}

/// The escapes themselves, because "the ghost's colour, reversed" is a claim about bytes.
///
/// The correction span carries `7` *and* the same colour the arrow does. Reverse alone would leave
/// it whatever the terminal's default foreground happens to be, which is not the ghost's grey and
/// is a different block on every colour scheme.
#[test]
fn the_correction_carries_the_ghost_colour_and_the_reverse() {
    let s = crate::theme::Syntax::default();
    assert_eq!(
        annotate(
            "lsvlk",
            "lsblk",
            &s.autosuggestion,
            &s.repair,
            Depth::Ansi256
        ),
        "\x1b[38;5;240m-> \x1b[0m\x1b[7;38;5;240m[lsblk]\x1b[0m"
    );
}

/// **`=name` is a command name too.** `=` is the shorthand for where a command lives, so its tail
/// is respelled the same way a first word is — and because the first word of `ls =somehig` is
/// spelled perfectly, [`super::spelling`] never looks at the rest of the line.
#[test]
fn an_equals_argument_is_respelled_as_a_command() {
    let (_dir, path) = path();
    assert_eq!(
        of("ls =lsvlk", &path, &nothing_is_known()).as_deref(),
        Some("ls =lsblk")
    );
    // Anywhere on the line, and the rest of it is untouched.
    assert_eq!(
        of("cat =carog -n", &path, &nothing_is_known()).as_deref(),
        Some("cat =cargo -n")
    );
    // In command position as well: `=lsvlk` is what would have run.
    assert_eq!(
        of("=lsvlk", &path, &nothing_is_known()).as_deref(),
        Some("=lsblk")
    );
}

/// The same three rules the first word follows: a name that resolves is not a mistake, a name that
/// begins one is unfinished, and a name with a `/` is not a name the shorthand resolves at all.
#[test]
fn an_equals_argument_that_is_not_wrong_is_left_alone() {
    let (_dir, path) = path();
    for typed in [
        "ls =cargo",
        "ls =car",
        "ls =/usr/bin/nosuch",
        "ls =",
        "ls a=b",
    ] {
        assert!(
            of(typed, &path, &nothing_is_known()).is_none(),
            "{typed:?} should not be corrected"
        );
    }
}

/// **Grammar is not part of a name.** `(cd /tmp && pwd)` found `mcd` two edits from `(cd`, and
/// accepting the correction rewrote the line into `mcd /tmp && pwd)` — a different program, with
/// the subshell silently gone. The same path proposed `[als]` for `!ls`.
#[test]
fn shell_grammar_is_never_respelled() {
    let (_dir, path) = path();
    for typed in [
        "(cd /tmp && pwd)",
        "(lsvlk)",
        "{ lsvlk; }",
        "!lsvlk",
        "! lsvlk",
        "}lsvlk",
        ")lsvlk",
    ] {
        assert!(
            of(typed, &path, &nothing_is_known()).is_none(),
            "{typed:?} must not be rewritten: the punctuation is grammar, not a name"
        );
    }
    // And the ordinary case still works, so the guard is not simply switching the feature off.
    assert_eq!(
        of("lsvlk", &path, &nothing_is_known()).as_deref(),
        Some("lsblk")
    );
}
