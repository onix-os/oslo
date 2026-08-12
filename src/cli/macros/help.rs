//! `oslo macros` help rendering.
//!
//! The same shape as `history`'s and `plugin`'s, to the letter: one `Sub` per subcommand, an
//! overview built from `row`, per-subcommand help behind `--help`, and notes wrapped to the
//! terminal. Three subcommand helps that read differently are three things to learn instead of one.

use crate::cli::help::{Paint, row};
use std::fmt::Write as _;

pub(super) struct Sub {
    pub name: &'static str,
    pub args: &'static str,
    pub about: &'static str,
    pub flags: &'static [(&'static str, &'static str)],
    pub note: &'static str,
}

pub(super) const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "add",
        args: "NAME [BODY]",
        about: "store an alias, an abbreviation, a function or a script",
        flags: &[
            (
                "--abbrev",
                "an abbreviation: expanded into the line as you type it",
            ),
            ("--func", "a function: opens your editor, shell or Lua"),
            (
                "--script",
                "a script: opens your editor, any language, with a shebang",
            ),
            (
                "--edit",
                "open an alias in the editor instead of taking it as an argument",
            ),
        ],
        note: "With no BODY and no flag, the editor opens. `--func` and `--script` always open it, \
               because neither fits on a command line. An alias and an abbreviation differ in what \
               they touch: an alias replaces the word before the line is parsed and a script never \
               sees what you typed, while an abbreviation is expanded *into your line*, so what \
               runs is what you can see and what the history records.",
    },
    Sub {
        name: "remove",
        args: "NAME",
        about: "forget one",
        flags: &[
            ("--abbrev", "remove the abbreviation of that name"),
            ("--func", "remove the function"),
            ("--script", "remove the script"),
        ],
        note: "Without a flag it removes the alias. A name may be more than one kind at once, so \
               removing without saying which removes only the alias — `oslo macros show NAME` \
               lists every kind that name has. Removing one that shadowed a `config.lua` alias \
               puts the configured one back on the next shell.",
    },
    Sub {
        name: "show",
        args: "[NAME]",
        about: "the list, narrowed as you type",
        flags: &[
            (
                "--plain",
                "one line each on stdout, for a script rather than a person",
            ),
            ("--edit", "open the one you pick in your editor"),
        ],
        note: "A function or a script is many lines and a list of many-line entries is not a list, \
               so each is flattened to `kind  name  first line`. With a NAME it prints that one in \
               full instead. An entry that shadows an alias your configuration defines is marked, \
               because finding that out from a list is fine and finding it out by wondering why \
               your config stopped working is not.",
    },
    Sub {
        name: "export",
        args: "[FILE]",
        about: "write every entry out as text",
        flags: &[],
        note: "A database is not a dotfiles repository: an alias in `config.lua` is \
               version-controlled, diffable and copied to a new machine with the rest of your \
               configuration, and one in here is none of those. This is the way back out. Without \
               a FILE it writes to stdout.",
    },
    Sub {
        name: "import",
        args: "[FILE]",
        about: "read entries back in",
        flags: &[("--replace", "forget everything stored first")],
        note: "The format `export` writes. Without `--replace` an entry with a name already \
               stored overwrites that one and the rest are left alone, so importing twice is the \
               same as importing once.",
    },
];

/// The overview.
pub fn text(paint: Paint) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "{}", paint.head("USAGE"));
    let _ = writeln!(
        text,
        "  {} {} {} {}",
        paint.key("oslo"),
        paint.key("macros"),
        paint.slot("<subcommand>"),
        paint.slot("[argument]...")
    );
    let _ = writeln!(text, "\n{}", paint.head("SUBCOMMANDS"));
    for sub in SUBCOMMANDS {
        text.push_str(&row(sub.name, paint.key(sub.name), sub.about));
    }
    let _ = writeln!(
        text,
        "\n  {}",
        paint.dim("`oslo macros <subcommand> --help` for that subcommand's arguments.")
    );
    let _ = writeln!(
        text,
        "  {}",
        paint.dim("Aliases and abbreviations reach a shell when it starts; a function or a script")
    );
    let _ = writeln!(
        text,
        "  {}",
        paint.dim("is found when you call it, after $PATH has already failed.")
    );
    text
}

/// Help for one subcommand.
pub fn subcommand(name: &str, paint: Paint) -> Option<String> {
    let sub = SUBCOMMANDS.iter().find(|sub| sub.name == name)?;
    let mut text = String::new();
    let _ = writeln!(text, "{}", paint.head("USAGE"));
    let _ = write!(
        text,
        "  {} {} {}",
        paint.key("oslo"),
        paint.key("macros"),
        paint.key(sub.name)
    );
    if !sub.args.is_empty() {
        let _ = write!(text, " {}", paint.slot(sub.args));
    }
    let _ = writeln!(text, "\n\n  {}", sub.about);

    if !sub.flags.is_empty() {
        let _ = writeln!(text, "\n{}", paint.head("ARGUMENTS"));
        for (flag, about) in sub.flags {
            text.push_str(&row(flag, paint.key(flag), about));
        }
    }
    if !sub.note.is_empty() {
        text.push('\n');
        for line in wrapped(sub.note) {
            let _ = writeln!(text, "  {}", paint.dim(&line));
        }
    }
    Some(text)
}

/// Wraps a note to the terminal width.
fn wrapped(note: &str) -> Vec<String> {
    const INDENT: usize = 2;
    let width = oslo::ui::dropdown::width::terminal_cols()
        .saturating_sub(INDENT + 2)
        .clamp(32, 96);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in note.split_whitespace() {
        let would_be = line.chars().count() + usize::from(!line.is_empty()) + word.chars().count();
        if would_be > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_subcommand_has_help_and_a_reason() {
        for sub in SUBCOMMANDS {
            assert!(
                subcommand(sub.name, Paint::plain()).is_some(),
                "{} has no help",
                sub.name
            );
            assert!(!sub.about.is_empty(), "{} says nothing", sub.name);
            assert!(sub.note.len() > 40, "{}'s note explains nothing", sub.name);
        }
    }

    #[test]
    fn a_name_nobody_listed_has_no_help() {
        assert!(subcommand("nonsense", Paint::plain()).is_none());
    }

    /// The overview names every subcommand, or one is reachable and undocumented.
    #[test]
    fn the_overview_lists_them_all() {
        let overview = text(Paint::plain());
        for sub in SUBCOMMANDS {
            assert!(overview.contains(sub.name), "{} is missing", sub.name);
        }
    }
}
