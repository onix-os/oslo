//! `on-report` — letting a config draw what the shell was going to draw.
//!
//! Five subsystems print a block of their own: the directory environment, the job notice, the
//! slow-command notice, `chain`, and `time`. Each decided what it looked like in Rust, and a config
//! could turn one off but never change it.
//!
//! ```lua
//! oslo.on.on_report(function(r)
//!   if r.kind == "direnv" and r.state == "loaded" then
//!     local b = oslo.ui.block("direnv " .. r.owner)
//!     for _, v in ipairs(r.changed) do b:row(v.change, v.name) end
//!     b:done()
//!     return true          -- handled; oslo prints nothing
//!   end
//! end)
//! ```
//!
//! # One hook, not five
//!
//! A plugin author learns one name, and a new reporter is a new `kind` rather than a new hook row
//! and a new fire site. Handlers that care about one kind cost nothing for the rest, because
//! returning nothing leaves oslo's own rendering exactly as it was.
//!
//! # Why this is not `on-job-finish`
//!
//! That hook already exists and stays. It answers "this happened" — a handler that logs a finished
//! job attaches there. This one answers "how should this look", and merging them would mean a
//! handler that merely *logged* a job silently *suppressed* its notice.
//!
//! # Where it may fire
//!
//! **Only from a site where the shell's state is free.** A report has to be answered before the
//! default is drawn, so it cannot be deferred the way a notifying hook can — and an answering hook
//! that fires while `Environment` is locked hands the handler a shell it cannot use.
//! `environments::arrive` runs in the read loop with nothing held, which is why the directory
//! environment is the one wired here. A reporter that fires from inside a builtin needs its fire
//! site moved before it can join, not a queue.

use oslo::direnv::Event;
use oslo::direnv::diff::Change;
use oslo::lua::api::hooks;
use oslo::lua::eval::value::{Table, Value};

/// Whether a config drew this event itself.
///
/// `false` — carry on and draw the default — whenever nothing is attached, nothing answered, or a
/// handler raised. **A broken plugin must not make the shell silent**, which is what returning
/// `true` on an error would do.
pub fn handled(event: &Event) -> bool {
    // The payload is a table of thirty-five names on a Nix arrival, so it is not built at all
    // unless somebody is listening. One relaxed load answers that.
    if !hooks::watched(hooks::at::ON_REPORT) {
        return false;
    }
    matches!(
        oslo::lua::engine::answer_hook_with(hooks::at::ON_REPORT, vec![direnv_fields(event)]),
        Some(Value::Bool(true))
    )
}

/// The event as the table a handler walks.
fn direnv_fields(event: &Event) -> Value {
    let mut fields = Table::new();
    fields.set(Value::str("kind"), Value::str("direnv"));
    match event {
        Event::Loaded {
            owner,
            changed,
            aliases,
        } => {
            fields.set(Value::str("state"), Value::str("loaded"));
            fields.set(Value::str("owner"), Value::str(owner.display().to_string()));
            fields.set(Value::str("changed"), changes(changed));
            fields.set(Value::str("aliases"), changes(aliases));
        }
        Event::Unloaded { owner } => {
            fields.set(Value::str("state"), Value::str("unloaded"));
            fields.set(Value::str("owner"), Value::str(owner.display().to_string()));
        }
        Event::Blocked { path } => {
            fields.set(Value::str("state"), Value::str("blocked"));
            fields.set(Value::str("owner"), Value::str(path.display().to_string()));
        }
        Event::Denied { path } => {
            fields.set(Value::str("state"), Value::str("denied"));
            fields.set(Value::str("owner"), Value::str(path.display().to_string()));
        }
        Event::Failed { path, problem } => {
            fields.set(Value::str("state"), Value::str("failed"));
            fields.set(Value::str("owner"), Value::str(path.display().to_string()));
            fields.set(Value::str("problem"), Value::str(problem));
        }
    }
    Value::table(fields)
}

/// `{ {name = "PATH", change = "changed"}, … }`.
///
/// A list of records rather than three separate lists, so a handler that does not care which kind
/// a name is can walk one thing — and one that does care reads a field rather than knowing which
/// of three keys it came from.
fn changes(items: &[(String, Change)]) -> Value {
    let mut list = Table::new();
    for (i, (name, change)) in items.iter().enumerate() {
        let mut row = Table::new();
        row.set(Value::str("name"), Value::str(name));
        row.set(
            Value::str("change"),
            Value::str(match change {
                Change::Added => "added",
                Change::Modified => "changed",
                Change::Removed => "removed",
            }),
        );
        list.set(Value::int(i as i64 + 1), Value::table(row));
    }
    Value::table(list)
}
