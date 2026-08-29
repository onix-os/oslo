//! The directory environment's `on-report` payload.
//!
//! The hook itself is `oslo_ui::report`, in the library, because four of the five
//! reporters live there. This is only the part that knows what a `direnv::Event` contains — it
//! stays in the binary because the *rendering* it stands in front of does too.

use oslo_base::value::Value;
use oslo_shell::direnv::Event;
use oslo_shell::direnv::diff::Change;
use oslo_ui::report::{self, rows, text};

/// Whether a config drew this event itself. See `oslo_ui::report::handled`.
pub fn handled(event: &Event) -> bool {
    if !report::watched() {
        return false;
    }
    report::handled("direnv", fields(event))
}

fn fields(event: &Event) -> Vec<(&'static str, Value)> {
    match event {
        Event::Loaded {
            owner,
            changed,
            aliases,
            functions,
        } => vec![
            ("state", text("loaded")),
            ("owner", text(&owner.display().to_string())),
            ("changed", changes(changed)),
            ("aliases", changes(aliases)),
            // Names, not changes: a name set is all a function diff can measure. See
            // `direnv::Event::Loaded`.
            (
                "functions",
                rows(
                    functions
                        .iter()
                        .map(|name| vec![("name", text(name))])
                        .collect(),
                ),
            ),
        ],
        Event::Unloaded { owner } => vec![
            ("state", text("unloaded")),
            ("owner", text(&owner.display().to_string())),
        ],
        Event::Blocked { path } => vec![
            ("state", text("blocked")),
            ("owner", text(&path.display().to_string())),
        ],
        Event::Denied { path } => vec![
            ("state", text("denied")),
            ("owner", text(&path.display().to_string())),
        ],
        Event::Failed { path, problem } => vec![
            ("state", text("failed")),
            ("owner", text(&path.display().to_string())),
            ("problem", text(problem)),
        ],
    }
}

/// `{ {name = "PATH", change = "changed"}, … }`.
///
/// A list of records rather than three lists, so a handler that does not care which kind a name is
/// walks one thing — and one that does care reads a field rather than knowing which of three keys
/// it came out of.
fn changes(items: &[(String, Change)]) -> Value {
    rows(
        items
            .iter()
            .map(|(name, change)| {
                vec![
                    ("name", text(name)),
                    (
                        "change",
                        text(match change {
                            Change::Added => "added",
                            Change::Modified => "changed",
                            Change::Removed => "removed",
                        }),
                    ),
                ]
            })
            .collect(),
    )
}
