//! History command help rendering.

use crate::cli::help::{Paint, row};
use std::fmt::Write as _;

/// A history subcommand help entry.
struct Sub {
    name: &'static str,
    /// Argument syntax.
    args: &'static str,
    /// Overview description.
    about: &'static str,
    /// Supported flags.
    flags: &'static [(&'static str, &'static str)],
    /// Additional usage note.
    note: &'static str,
}

const JSON: (&str, &str) = ("--json", "answer as JSON, valid even when empty");

const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "path",
        args: "",
        about: "print the current profile's database path",
        flags: &[],
        note: "Writes nothing and creates nothing.",
    },
    Sub {
        name: "status",
        args: "[FILE] [--json]",
        about: "report what a history database holds",
        flags: &[JSON],
        note: "Path, schema version, file size, event and tombstone counts.",
    },
    Sub {
        name: "list",
        args: "[QUERY] [-n N] [--oldest] [--json] [--null]",
        about: "list events, newest first",
        flags: &[
            ("-n N", "how many to show"),
            ("--oldest", "oldest first instead"),
            JSON,
            ("--null", "terminate text records with NUL, for xargs -0"),
        ],
        note: "The optional query is a plain substring. For anything finer, use `search`.",
    },
    Sub {
        name: "search",
        args: "QUERY [-n N] [FILTERS] [--json] [--null]",
        about: "search events by text and by what surrounded them",
        flags: &[
            ("--exact", "the whole command line equals QUERY"),
            ("--prefix", "the command line starts with QUERY"),
            (
                "--contains",
                "the command line contains QUERY (the default)",
            ),
            ("--host HOST", "only commands run on that host"),
            ("--cwd PATH", "only commands run in that directory"),
            ("--status N", "only commands that exited with N"),
            ("--since DURATION", "only commands newer than, e.g. 7d"),
            ("--before DURATION", "only commands older than"),
            ("-n N", "how many to show"),
            JSON,
            ("--null", "terminate text records with NUL, for xargs -0"),
        ],
        note: "",
    },
    Sub {
        name: "show",
        args: "EVENT_ID [--json]",
        about: "show one event in full",
        flags: &[JSON],
        note: "Takes the stable event ID that `list` and `search` print, never a local row number.",
    },
    Sub {
        name: "stats",
        args: "[--host HOST] [--since DURATION] [--json]",
        about: "summarise what has been run",
        flags: &[
            ("--host HOST", "only commands run on that host"),
            ("--since DURATION", "only commands newer than, e.g. 30d"),
            JSON,
        ],
        note: "Counts, successes and failures, hosts, directories, durations and the time range.",
    },
    Sub {
        name: "verify",
        args: "[FILE] [--json]",
        about: "check a database for damage",
        flags: &[JSON],
        note: "Read-only, and strictly so: it never creates, renames, replaces or migrates FILE.",
    },
    Sub {
        name: "sync",
        args: "OTHER | FILE1 FILE2 [--dry-run] [--json]",
        about: "merge two history databases, both ways",
        flags: &[
            ("--dry-run", "say what would change and change nothing"),
            JSON,
        ],
        note: "With one argument, the current profile and OTHER. With two, those two files — and \
               the order picks no winner: resolution is deterministic either way, and syncing \
               again converges rather than duplicating.",
    },
    Sub {
        name: "delete",
        args: "EVENT_ID... [--yes]",
        about: "remove specific events everywhere",
        flags: &[("--yes", "do not ask")],
        note: "A tombstone, not an erasure: that is what makes the removal survive the next sync \
               instead of the other machine putting it back.",
    },
    Sub {
        name: "clear",
        args: "--yes",
        about: "remove every visible event everywhere",
        flags: &[("--yes", "required; there is no prompt to skip")],
        note: "Tombstones the lot, and every machine you sync with will drop them too.",
    },
    Sub {
        name: "prune",
        args: "[--dry-run] [--yes]",
        about: "local retention maintenance",
        flags: &[
            ("--dry-run", "say what would go and remove nothing"),
            ("--yes", "do not ask"),
        ],
        note: "Local only. Trimming this machine's copy is not a decision about anybody \
               else's, so this makes no tombstones and syncs nothing away.",
    },
    Sub {
        name: "export",
        args: "[FILE|-] [--format jsonl|text]",
        about: "write history out in a portable form",
        flags: &[(
            "--format jsonl|text",
            "JSONL keeps everything; text is command lines only",
        )],
        note: "`-` writes to stdout. JSONL is the one to keep: text cannot be imported as the \
               same events.",
    },
    Sub {
        name: "import",
        args: "FILE [--dry-run]",
        about: "read history back in",
        flags: &[("--dry-run", "say what would arrive and store nothing")],
        note: "JSONL keeps the event IDs it was exported with, so importing the same file twice \
               changes nothing the second time. Text input has no IDs and becomes new events.",
    },
    Sub {
        name: "backup",
        args: "FILE",
        about: "copy the database safely while it is in use",
        flags: &[],
        note: "A consistent snapshot, not a file copy — copying a live database is how you get one \
               that half-opens.",
    },
];

/// Renders the history subcommand overview.
pub fn text(paint: Paint) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "{}", paint.head("USAGE"));
    let _ = writeln!(
        text,
        "  {} {} {} {}",
        paint.key("oslo"),
        paint.key("history"),
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
        paint.dim("`oslo history <subcommand> --help` for that subcommand's arguments.")
    );
    text
}

/// Renders help for one history subcommand.
pub fn subcommand(name: &str, paint: Paint) -> Option<String> {
    let sub = SUBCOMMANDS.iter().find(|sub| sub.name == name)?;
    let mut text = String::new();
    let _ = writeln!(text, "{}", paint.head("USAGE"));
    let _ = write!(
        text,
        "  {} {} {}",
        paint.key("oslo"),
        paint.key("history"),
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
