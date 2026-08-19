//! What `=name` and `@name` retarget to.
//!
//! Both used to reach the completer as ordinary words: `ls =gr` looked for a *file* called `gr`
//! and `cd @wo` looked for one called `@wo`, so Tab did nothing on either.

use super::{at_segment, equals_segment, escaped_segment};
use crate::words::current_word;
use std::collections::HashMap;

/// **The escape survives the completion.** `\rm` asks for the program rather than oslo's builtin,
/// and replacing the whole word deleted the backslash — turning the line silently back into the
/// builtin it was written to avoid.
#[test]
fn an_escaped_command_keeps_its_backslash() {
    for (line, escape) in [("\\r", 1), ("\\\\r", 2)] {
        let word = escaped_segment(current_word(line, line.len()));
        assert_eq!(word.text, "r", "{line}");
        assert_eq!(word.stem, "r", "{line}");
        assert_eq!(word.start, escape, "{line}");
        assert!(word.command_position, "{line}");
        assert_eq!(word.carried, 0, "{line}");
    }
}

/// Only in command position, which is the only place the escape means anything — and never for a
/// path, which the escape does not apply to.
#[test]
fn an_escape_elsewhere_is_left_alone() {
    for line in ["echo \\r", "\\./x", "\\"] {
        let before = current_word(line, line.len());
        let (start, text) = (before.start, before.text);
        let after = escaped_segment(before);
        assert_eq!((after.start, after.text), (start, text), "{line}");
    }
}

/// `=name` completes the command the shorthand resolves, not a file.
#[test]
fn equals_retargets_at_a_command_name() {
    let line = "ls =gr";
    let word = equals_segment(current_word(line, line.len()));

    assert_eq!(word.text, "gr");
    assert_eq!(word.stem, "gr");
    assert!(word.command_position, "=gr names a command");
    assert_eq!(word.start, line.find("gr").unwrap());
    assert_eq!(word.carried, 0);
}

/// It applies in command position too: `=grep foo` runs grep.
#[test]
fn equals_retargets_the_first_word_as_well() {
    let line = "=gr";
    let word = equals_segment(current_word(line, line.len()));

    assert_eq!(word.text, "gr");
    assert!(word.command_position);
    assert_eq!(word.start, 1);
}

/// An `=` that is not the shorthand is left for the ordinary word break to handle.
#[test]
fn equals_leaves_everything_else_alone() {
    for line in ["tar --file=arch", "FOO=bar", "ls =/usr/bi", "ls 'a=b"] {
        let before = current_word(line, line.len());
        let (start, text) = (before.start, before.text);
        let after = equals_segment(before);
        assert_eq!((after.start, after.text), (start, text), "{line}");
    }
}

/// `@name/tail` reads the directory the name stands for, and writes back only the tail.
#[test]
fn at_retargets_under_the_registered_directory() {
    oslo_base::dirs::set_named_dirs(HashMap::from([(
        "work".to_string(),
        "/home/u/work".to_string(),
    )]));

    let line = "cd @work/src/ma";
    let word = at_segment(current_word(line, line.len()));

    assert_eq!(word.text, "/src/ma");
    assert_eq!(word.stem, "/home/u/work/src/ma");
    // The resolved directory is context, not text to replace: without this the completion writes
    // `@work/home/u/work/src/main.rs`.
    assert_eq!(word.carried, "/home/u/work".len());
    assert_eq!(word.start, line.find("/src/ma").unwrap());

    oslo_base::dirs::set_named_dirs(HashMap::new());
}

/// A name nobody registered expands to itself, so it completes against the filesystem.
#[test]
fn at_leaves_an_unregistered_name_alone() {
    oslo_base::dirs::set_named_dirs(HashMap::new());

    for line in ["cd @nowhere/x", "cd @work", "cd plain/x"] {
        let before = current_word(line, line.len());
        let (start, text) = (before.start, before.text);
        let after = at_segment(before);
        assert_eq!((after.start, after.text), (start, text), "{line}");
    }
}
