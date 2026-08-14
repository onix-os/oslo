//! What `oslo history --help` says.
//!
//! The rows only; every decision about how they are drawn is [`crate::cli::help::menu`]'s, which is
//! what makes this page and `oslo secret --help` the same page with different words in it.

use crate::cli::help::Paint;
use crate::cli::help::menu::{CALL, Menu, SUBCOMMANDS as HEADING, Sub};

const JSON: (&str, &str) = ("--json", "answer as JSON, valid even when empty");

pub(crate) const MENU: Menu = Menu {
    path: &["history"],
    call: CALL,
    heading: HEADING,
    subs: SUBCOMMANDS,
    notes: &[],
    nested: &[],
};

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

/// The overview.
pub fn text(paint: Paint) -> String {
    MENU.overview(paint)
}

#[cfg(test)]
mod tests;
