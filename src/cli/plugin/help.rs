//! Plugin command help rendering.
//!
//! The same shape as `history`'s, deliberately and to the letter: one `Sub` per subcommand, an
//! overview built from `row`, per-subcommand help behind `--help`, and notes wrapped to the
//! terminal. Two subcommand helps that read differently are two things to learn instead of one.

use crate::cli::help::{Paint, row};
use std::fmt::Write as _;

/// A plugin subcommand help entry.
pub(super) struct Sub {
    pub name: &'static str,
    /// Argument syntax.
    pub args: &'static str,
    /// Overview description.
    pub about: &'static str,
    /// Supported flags.
    pub flags: &'static [(&'static str, &'static str)],
    /// Additional usage note.
    pub note: &'static str,
}

pub(super) const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "list",
        args: "",
        about: "what is installed, and whether it still matches what you allowed",
        flags: &[],
        note: "A plugin whose files have changed since you allowed them is marked CHANGED and \
               will not load until `oslo plugin allow` records the new hash.",
    },
    Sub {
        name: "doctor",
        args: "[NAME]",
        about: "check what is installed, and why a plugin might not be working",
        flags: &[],
        note: "With a name, the plugin is loaded and its own checks are asked too — the only ones \
               that know whether the program it shells out to is installed. Without one, nothing is \
               loaded.",
    },
    Sub {
        name: "install",
        args: "PATH|GIT [--yes]",
        about: "copy or clone a plugin in, after showing what it reserves",
        flags: &[(
            "--yes",
            "do not ask before allowing it to run, for a script",
        )],
        note: "A git source must name a revision — `github:user/repo@v1.0` — because a branch \
               would be a different plugin tomorrow and the trust hash would refuse it every \
               morning. Nothing of the plugin runs during an install: its manifest is read in an \
               interpreter that cannot reach the shell, and the names it would reserve are shown \
               before you decide.",
    },
    Sub {
        name: "remove",
        args: "NAME",
        about: "delete a plugin; its database is left where it is",
        flags: &[],
        note: "The database is your data, not the plugin's. Reinstalling hands the same one back.",
    },
    Sub {
        name: "allow",
        args: "NAME",
        about: "record what a plugin hashes to now, after an update",
        flags: &[],
        note: "Look at what changed first. An update is somebody else's new code arriving on your \
               machine, which is the whole reason it stopped loading.",
    },
];

/// Renders the plugin subcommand overview.
pub fn text(paint: Paint) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "{}", paint.head("USAGE"));
    let _ = writeln!(
        text,
        "  {} {} {} {}",
        paint.key("oslo"),
        paint.key("plugin"),
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
        paint.dim("`oslo plugin <subcommand> --help` for that subcommand's arguments.")
    );
    let _ = writeln!(
        text,
        "  {}",
        paint.dim("A plugin's commands run in the interactive shell, never in a script.")
    );
    text
}

/// Renders help for one plugin subcommand.
pub fn subcommand(name: &str, paint: Paint) -> Option<String> {
    let sub = SUBCOMMANDS.iter().find(|sub| sub.name == name)?;
    let mut text = String::new();
    let _ = writeln!(text, "{}", paint.head("USAGE"));
    let _ = write!(
        text,
        "  {} {} {}",
        paint.key("oslo"),
        paint.key("plugin"),
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
mod tests;
