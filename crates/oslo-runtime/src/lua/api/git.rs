//! `oslo.git` — what a repository says about itself, without running `git`.
//!
//! ```lua
//! oslo.prompt.left = function()
//!   local head = oslo.git.head()
//!   if not head then return "$ " end
//!   local mark = head.detached and ("@" .. head.commit:sub(1, 7)) or head.branch
//!   local doing = oslo.git.operation()          -- "rebase" while one is part-way through
//!   local ahead = oslo.git.upstream()           -- "origin/main", or nil if it tracks nothing
//!   local kept  = oslo.git.stash()              -- how many stash entries there are
//!   return mark .. (doing and (" " .. doing) or "") .. " $ "
//! end
//! ```
//!
//! Every call here is one to three small file reads and no process — see [`oslo_ui::git`] for why
//! that matters at prompt-draw rates, and for why `dirty` and `ahead`/`behind` are deliberately not
//! among them.

use super::util::{ok, put, record};
use oslo_base::value::{Table, Value};

/// Build the `oslo.git` table.
pub fn build() -> Value {
    let mut git = Table::new();

    // oslo.git.branch() -> "main", a short hash when detached, or nil outside a repository.
    //
    // Kept as it was — the one call that answers a *label* rather than a fact, because that is
    // what almost every prompt wants and `head()` is three lines to write out by hand.
    put(&mut git, "branch", |_, _| {
        ok(text(oslo_ui::prompt::git_branch()))
    });

    // oslo.git.root() -> the working tree's top directory, or nil.
    put(&mut git, "root", |_, _| {
        ok(match oslo_ui::prompt::git_root() {
            Some(root) => Value::str(root.to_string_lossy()),
            None => Value::Nil,
        })
    });

    // oslo.git.dir() -> the real git directory, which in a linked worktree is not `root .. "/.git"`
    put(&mut git, "dir", |_, _| {
        ok(match oslo_ui::git::dir() {
            Some(dir) => Value::str(dir.to_string_lossy()),
            None => Value::Nil,
        })
    });

    // oslo.git.head() -> { branch = "main", commit = "…", detached = false }, or nil
    //
    // A table rather than three calls, because they are one question: three calls would read
    // `HEAD` three times and could answer about three different states if a `git checkout` landed
    // between them.
    put(&mut git, "head", |_, _| {
        ok(match oslo_ui::git::head() {
            Some(head) => record(vec![
                ("branch", text(head.branch.clone())),
                ("commit", text(head.commit.clone())),
                ("detached", Value::Bool(head.detached())),
            ]),
            None => Value::Nil,
        })
    });

    // oslo.git.operation() -> "rebase" | "merge" | "cherry-pick" | "revert" | "bisect", or nil
    put(&mut git, "operation", |_, _| {
        ok(match oslo_ui::git::operation() {
            Some(name) => Value::str(name),
            None => Value::Nil,
        })
    });

    // oslo.git.stash() -> how many entries the stash holds; 0 outside a repository
    //
    // A number rather than nil-or-number: "no stash" and "not a repository" both mean nothing is
    // stashed, and a prompt writing `if oslo.git.stash() > 0` should not have to know which.
    put(&mut git, "stash", |_, _| {
        ok(Value::int(oslo_ui::git::stash_count() as i64))
    });

    // oslo.git.upstream() -> "origin/main", or nil when the branch tracks nothing
    put(&mut git, "upstream", |_, _| {
        ok(text(oslo_ui::git::upstream()))
    });

    // oslo.git.tag() -> a tag pointing at HEAD, or nil
    put(&mut git, "tag", |_, _| {
        ok(text(oslo_ui::git::tag_at_head()))
    });

    Value::table(git)
}

/// An optional string as a Lua value.
fn text(value: Option<String>) -> Value {
    match value {
        Some(text) => Value::str(text),
        None => Value::Nil,
    }
}

#[cfg(test)]
#[path = "git/tests.rs"]
mod tests;
