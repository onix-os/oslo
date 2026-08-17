//! `oslo.history` — what the shell has been asked to do, as rows a config can read.
//!
//! ```lua
//! for _, c in ipairs(oslo.history.commands{ limit = 50 }) do
//!   print(c.runs, c.line, c.dir)
//! end
//!
//! -- Every command run in this repository, most-run first.
//! local root = oslo.git.root()
//! local mine = {}
//! for _, c in ipairs(oslo.history.commands()) do
//!   if c.root == root then mine[#mine + 1] = c end
//! end
//! table.sort(mine, function(a, b) return a.runs > b.runs end)
//!
//! oslo.history.forget("curl -H 'Authorization: Bearer …' …")
//! ```
//!
//! # Why reading it is worth an API
//!
//! The finder over this is oslo's, and it is one opinion about what to show: newest first, folded
//! on the line, ranked by frecency. **Anybody else's opinion needs the rows.** "What do I run in
//! this project", "which of my commands have only ever failed", "what did I run yesterday that I
//! have not run since" are all one loop over a table, and none of them is a feature the shell
//! should have to grow.
//!
//! `history` the *file* cannot answer any of them — it is a list of lines. These rows carry the
//! directory, the count, the exit status, the session and the machine, because the tracker has been
//! writing all of that on every command since long before there was anything to read it with.
//!
//! # Forgetting, and why it is here rather than only on a key
//!
//! `oslo.history.forget(line)` is the finder's Delete key as a function. It takes the line out of
//! **every** directory and out of the log as well as the aggregate — a half-forgotten line is one
//! that comes back on the next start, which is the bug this call exists to not have. That makes it
//! worth having in a config: "forget anything matching this pattern" is a rule somebody wants
//! applied on every start, and a key press cannot be.
//!
//! # Only an interactive shell has a store
//!
//! The tracker is installed by the REPL and by nothing else, so **a script, an `oslo -c` and a
//! subshell all see an empty history** — they have nowhere to write to either, which is the same
//! decision seen from the other side. `commands()` answers an empty list rather than `nil` there,
//! so `for _, c in ipairs(oslo.history.commands())` is safe to write in a file that might be run
//! either way.

use super::util::{list, ok, opt_text, put, record, text};
use oslo_base::value::{Table, Value};

/// How many commands a bare `commands()` answers with.
///
/// The store folds to far fewer distinct lines than it holds runs, so this is generous rather than
/// a limit anybody meets — but it is bounded, because a config that asks for everything on a store
/// of a hundred thousand runs should not build a table it did not mean to.
const DEFAULT_LIMIT: usize = 1000;

/// Build the `oslo.history` table.
///
/// Merged into the settings namespace of the same name rather than replacing it — `oslo.history` is
/// already where the finder's settings are written, and two tables would mean two names for one
/// subject. See `super::extend`.
pub fn build() -> Table {
    let mut it = Table::new();

    // oslo.history.commands{ limit = 1000 } -> newest-first rows, folded on the line
    put(&mut it, "commands", |_, args| {
        let limit = match args.first() {
            Some(Value::Table(options)) => match options.borrow().get_str("limit") {
                Value::Number(n) => n.as_int().unwrap_or(0).max(0) as usize,
                _ => DEFAULT_LIMIT,
            },
            _ => DEFAULT_LIMIT,
        };
        let Some(track) = oslo_base::track::store() else {
            // A shell whose store would not open is a working shell. An empty list says "nothing
            // recorded", which is the truth here and is what a loop over it should do.
            return ok(list([]));
        };
        ok(list(track.commands(limit).into_iter().map(row)))
    });

    // oslo.history.forget(line, mode) -> how many rows went
    //
    // `mode` is "sh" or "lua" and defaults to "sh", because a line typed at the prompt is a shell
    // line unless the prompt was in Lua mode.
    put(&mut it, "forget", |_, args| {
        let line = text(&args, 1, "oslo.history.forget")?;
        let mode = opt_text(&args, 2, "oslo.history.forget")?.unwrap_or_else(|| "sh".into());
        let Some(track) = oslo_base::track::store() else {
            return ok(Value::int(0));
        };
        ok(Value::int(track.forget(&line, &mode) as i64))
    });

    it
}

/// One command as a Lua table.
///
/// Every field the tracker kept, named as it is named in Rust. A row that dropped `places` or
/// `worked` would be a row a caller has to go back to the shell for.
fn row(command: oslo_base::track::history::Command) -> Value {
    record(vec![
        ("line", Value::str(&command.line)),
        ("mode", Value::str(&command.mode)),
        ("runs", Value::int(command.runs)),
        // Unix seconds, so `os.date("%F", c.last_at)` renders it. A pre-formatted string here
        // would pick a format for everybody and make comparison a string comparison.
        ("last_at", Value::int(command.last_at)),
        ("dir", Value::str(&command.dir)),
        ("places", Value::int(command.places as i64)),
        ("worked", Value::Bool(command.worked)),
        ("session", Value::str(&command.session)),
        ("host", Value::str(&command.host)),
        (
            "root",
            match &command.root {
                Some(root) => Value::str(root),
                None => Value::Nil,
            },
        ),
    ])
}

#[cfg(test)]
#[path = "history/tests.rs"]
mod tests;
