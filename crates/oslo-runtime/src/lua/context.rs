//! What a prompt segment is told about the moment it is being drawn.
//!
//! A segment is a function, so it *could* look everything up itself — `oslo.fs.cwd()`,
//! `oslo.git.branch()`, and so on. The context exists anyway for two reasons. The facts here are
//! ones the shell already knows and a segment cannot get at all (the exit status of the last
//! command, how long it took, which language the prompt is reading), and the ones it could look up
//! are the expensive ones: with five segments each calling `git branch`, a prompt runs `git` five
//! times per keystroke.
//!
//! Gathered once per prompt and handed to every segment. Mirrors the `ctx` hexe passes its own
//! segments, so a config moves between the two without rewriting.

use oslo_base::value::{Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// The facts a prompt is drawn from.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Exit status of the command before this prompt.
    pub status: i32,
    /// How long it took, in milliseconds, or `None` when nothing has run yet.
    pub duration_ms: Option<u64>,
    /// The working directory, unabbreviated.
    pub cwd: String,
    /// The branch checked out here, if this is a work tree.
    pub branch: Option<String>,
    /// Who is logged in.
    pub user: String,
    /// This machine's short name.
    pub host: String,
    /// `"sh"` or `"lua"` — which language the line will be read as.
    pub language: String,
    /// `"I"`, `"N"` or `"R"` when vi mode is on.
    pub vimode: Option<String>,
    /// How wide the terminal is, so a segment can shorten itself rather than be dropped.
    pub cols: usize,
    /// How many jobs the shell is tracking.
    pub jobs: usize,
    /// Whether this is the continuation of an unfinished command.
    pub continuation: bool,
    /// The command that is *about to run*, when there is one.
    ///
    /// `None` at a prompt, since nothing is running. Set while a command is in flight, which is
    /// what makes `oslo.prompt.title` able to name it — a tab reading `cargo` is worth more than
    /// one reading the directory you started it from.
    pub command: Option<String>,
}

impl Context {
    /// The table a segment's `render(ctx)` is called with.
    pub fn to_lua(&self) -> Value {
        let mut t = Table::default();
        t.set(Value::str("status"), Value::int(self.status as i64));
        t.set(
            Value::str("duration_ms"),
            match self.duration_ms {
                Some(ms) => Value::int(ms as i64),
                None => Value::Nil,
            },
        );
        t.set(Value::str("cwd"), Value::str(&self.cwd));
        t.set(
            Value::str("branch"),
            match &self.branch {
                Some(b) => Value::str(b),
                None => Value::Nil,
            },
        );
        t.set(Value::str("user"), Value::str(&self.user));
        t.set(Value::str("host"), Value::str(&self.host));
        t.set(Value::str("language"), Value::str(&self.language));
        t.set(
            Value::str("vimode"),
            match &self.vimode {
                Some(m) => Value::str(m),
                None => Value::Nil,
            },
        );
        t.set(Value::str("cols"), Value::int(self.cols as i64));
        t.set(Value::str("jobs"), Value::int(self.jobs as i64));
        t.set(Value::str("continuation"), Value::Bool(self.continuation));
        t.set(
            Value::str("command"),
            match &self.command {
                Some(text) => Value::str(text),
                None => Value::Nil,
            },
        );
        // `ok` reads better than `status == 0` in the common case, and is the check almost every
        // prompt makes.
        t.set(Value::str("ok"), Value::Bool(self.status == 0));
        Value::Table(Rc::new(RefCell::new(t)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &Rc<RefCell<Table>>, key: &str) -> Option<String> {
        match t.borrow().get(&Value::str(key)) {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        }
    }
    fn int(t: &Rc<RefCell<Table>>, key: &str) -> Option<i64> {
        match t.borrow().get(&Value::str(key)) {
            Value::Number(n) => n.as_int(),
            _ => None,
        }
    }
    fn boolean(t: &Rc<RefCell<Table>>, key: &str) -> Option<bool> {
        match t.borrow().get(&Value::str(key)) {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }
    fn is_nil(t: &Rc<RefCell<Table>>, key: &str) -> bool {
        matches!(t.borrow().get(&Value::str(key)), Value::Nil)
    }
    fn table_of(ctx: &Context) -> Rc<RefCell<Table>> {
        match ctx.to_lua() {
            Value::Table(t) => t,
            _ => panic!("a context is a table"),
        }
    }

    /// Every field reaches Lua under the name the documentation gives it. A segment reading
    /// `ctx.branch` and silently getting `nil` because the field was named something else is the
    /// failure this guards.
    #[test]
    fn the_context_carries_every_field_into_lua() {
        let ctx = Context {
            status: 1,
            duration_ms: Some(1500),
            cwd: "/tmp/x".to_string(),
            branch: Some("develop".to_string()),
            user: "bo".to_string(),
            host: "tron".to_string(),
            language: "lua".to_string(),
            vimode: Some("N".to_string()),
            cols: 120,
            jobs: 2,
            continuation: false,
            command: Some("cargo build".to_string()),
        };
        let t = table_of(&ctx);
        assert_eq!(int(&t, "status"), Some(1));
        assert_eq!(boolean(&t, "ok"), Some(false), "status 1 is not ok");
        assert_eq!(int(&t, "duration_ms"), Some(1500));
        assert_eq!(text(&t, "cwd").as_deref(), Some("/tmp/x"));
        assert_eq!(text(&t, "branch").as_deref(), Some("develop"));
        assert_eq!(text(&t, "user").as_deref(), Some("bo"));
        assert_eq!(text(&t, "host").as_deref(), Some("tron"));
        assert_eq!(text(&t, "language").as_deref(), Some("lua"));
        assert_eq!(text(&t, "vimode").as_deref(), Some("N"));
        assert_eq!(int(&t, "cols"), Some(120));
        assert_eq!(int(&t, "jobs"), Some(2));
        assert_eq!(boolean(&t, "continuation"), Some(false));
    }

    /// What is not known is `nil`, not an empty string: a segment testing `if ctx.branch then`
    /// must not draw an empty branch outside a work tree.
    #[test]
    fn what_is_unknown_is_nil() {
        let t = table_of(&Context::default());
        assert!(is_nil(&t, "branch"));
        assert!(is_nil(&t, "vimode"));
        assert!(is_nil(&t, "duration_ms"));
        assert_eq!(boolean(&t, "ok"), Some(true), "status 0 is ok");
    }
}
