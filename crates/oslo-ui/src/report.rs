//! `on-report` — letting a config draw what the shell was going to draw.
//!
//! Five subsystems print a block of their own: the directory environment, the job notice, the
//! slow-command notice, `chain`, and `time`. Each decided what it looked like in Rust, and a config
//! could turn one off but never change it.
//!
//! ```lua
//! oslo.on.on_report(function(r)
//!   if r.kind == "job" then
//!     oslo.ui.block(("[%s] %s"):format(r.id, r.text)):done()
//!     return true        -- handled; oslo prints nothing
//!   end
//! end)
//! ```
//!
//! # One hook, not five
//!
//! A plugin author learns one name, and a new reporter is a new `kind` rather than a new hook row
//! and a new fire site. A handler that cares about one kind costs nothing for the rest: returning
//! anything but `true` leaves oslo's own rendering exactly as it was.
//!
//! # Why this is not `on-job-finish`
//!
//! That hook exists and stays. It answers "this happened" — a handler that logs a finished job
//! attaches there, and it is a *notifying* hook, so it may be deferred to a moment when the shell
//! is idle and it can act. This one answers "how should this look", and merging them would mean a
//! handler that merely logged a job silently suppressed its notice.
//!
//! # What a handler may do, and where
//!
//! **It may always draw.** `oslo.ui.block` touches no shell state, so a handler that renders works
//! from every one of the five.
//!
//! **It may not always change the shell.** A report has to be answered before the default is
//! drawn, so unlike a notifying hook it cannot be deferred — and three of the five fire from inside
//! a builtin or the executor, with `Environment` locked:
//!
//! | kind | fires from | state |
//! |---|---|---|
//! | `direnv` | the read loop, after `cd` | free |
//! | `slow` | the read loop, after a command | free |
//! | `chain` | the `chain` builtin | **held** |
//! | `job` | the job reaper | **held** |
//! | `time` | `time`'s own report | **held** |
//!
//! From a held one, `oslo.env.set` raises with the message `borrow_env` gives — loudly, naming the
//! cause. That is the same contract `pre-cmd` and `pre-change-dir` already have, and the reason
//! every field a handler could want is passed in rather than looked up.

use oslo_base::hooks;
use oslo_base::value::{Table, Value};

/// Whether a config drew this report itself.
///
/// `false` — carry on and draw the default — whenever nothing is attached, nothing answered, or a
/// handler raised. **A broken plugin must not make the shell silent**, which is what treating an
/// error as "handled" would do.
///
/// The payload is not built unless something is listening: a directory arrival carries thirty-five
/// changed names, and one relaxed load is the price of a shell that has no handler.
pub fn handled(kind: &str, fields: Vec<(&str, Value)>) -> bool {
    if !hooks::watched(hooks::at::ON_REPORT) {
        return false;
    }
    let mut table = Table::new();
    table.set(Value::str("kind"), Value::str(kind));
    for (name, value) in fields {
        table.set(Value::str(name), value);
    }
    matches!(
        hooks::answer_hook_with(hooks::at::ON_REPORT, vec![Value::table(table)]),
        Some(Value::Bool(true))
    )
}

/// Whether anything is listening at all, for a caller whose payload is expensive to build.
pub fn watched() -> bool {
    hooks::watched(hooks::at::ON_REPORT)
}

/// A string field, spelled once so a caller does not have to name the value type.
pub fn text(value: &str) -> Value {
    Value::str(value)
}

/// An integer field.
pub fn int(value: i64) -> Value {
    Value::int(value)
}

/// A list of records, the shape every "and here is what each one did" field uses.
pub fn rows(items: Vec<Vec<(&str, Value)>>) -> Value {
    let mut list = Table::new();
    for (i, item) in items.into_iter().enumerate() {
        let mut row = Table::new();
        for (name, value) in item {
            row.set(Value::str(name), value);
        }
        list.set(Value::int(i as i64 + 1), Value::table(row));
    }
    Value::table(list)
}
