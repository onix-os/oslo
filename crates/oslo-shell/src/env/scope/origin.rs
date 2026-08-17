//! Where the builtin now running should say its diagnostics came from.
//!
//! # Why this is a thread-local and not an argument
//!
//! [`Environment::origin`] answers the question, and `cd` had been using it since the location
//! work landed. Every other builtin printed a hardcoded `oslo: `, so a script that failed on
//! `read`, `printf`, `ulimit` or `trap` named the shell and not the line — the same complaint the
//! location work was written to answer, still true for forty of the forty-two builtins.
//!
//! Fixing that by passing the origin down meant threading a `&str` through every helper in every
//! builtin, most of which do not otherwise have an `Environment` to hand: `parse`, `usage`,
//! `report`, and the private functions under them. That is a large diff whose every line is the
//! same line, and it makes the next builtin's author responsible for remembering.
//!
//! So the origin is published once, at the single point every builtin is dispatched through, and
//! read by whoever needs it. [`Environment::exec_custom_builtin`] is documented as "the only way a
//! builtin is ever invoked", which is what makes one write enough.
//!
//! # Saved and restored, not just set
//!
//! Builtins nest: `command`, `builtin`, `eval` and `source` all run another one, and `source` can
//! enter a *different file* on the way. A plain store would leave the outer builtin reporting the
//! inner one's file after it returned, so [`Published`] puts back whatever it displaced.
//!
//! [`Environment::origin`]: super::Environment::origin
//! [`Environment::exec_custom_builtin`]: super::Environment::exec_custom_builtin

use std::cell::RefCell;

thread_local! {
    /// What [`origin_now`] answers. Per thread, because the test binaries evaluate a script per thread
    /// and a process-wide origin would have one test's file named in another's diagnostics.
    static ORIGIN: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The prefix a builtin's diagnostics should carry.
///
/// `oslo: ` when nothing has published one — at a prompt, under `-c`, and in a unit test that
/// calls a builtin directly. That is the same answer [`Environment::origin`] gives in those cases,
/// so an unpublished origin is indistinguishable from a published one that had no file to name.
///
/// [`Environment::origin`]: super::Environment::origin
pub fn origin_now() -> String {
    ORIGIN.with(|origin| {
        let origin = origin.borrow();
        if origin.is_empty() {
            "oslo: ".to_string()
        } else {
            origin.clone()
        }
    })
}

/// An origin that is in force until this is dropped.
pub struct Published(String);

impl Published {
    /// Publish `origin` for as long as the returned guard lives.
    pub fn new(origin: String) -> Published {
        Published(ORIGIN.with(|slot| slot.replace(origin)))
    }
}

impl Drop for Published {
    fn drop(&mut self) {
        ORIGIN.with(|slot| *slot.borrow_mut() = std::mem::take(&mut self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing published is the prompt's answer, which is what a direct unit test sees.
    #[test]
    fn an_unpublished_origin_is_the_shell_itself() {
        assert_eq!(origin_now(), "oslo: ");
    }

    #[test]
    fn a_published_origin_lasts_until_the_guard_is_dropped() {
        {
            let _held = Published::new("script.sh: line 4: ".to_string());
            assert_eq!(origin_now(), "script.sh: line 4: ");
        }
        assert_eq!(origin_now(), "oslo: ");
    }

    /// **The nesting case.** `source inner.sh` inside a builtin publishes the inner file; when it
    /// returns, the outer builtin must be back to naming its own.
    #[test]
    fn a_nested_origin_gives_the_outer_one_back() {
        let _outer = Published::new("outer.sh: line 1: ".to_string());
        {
            let _inner = Published::new("inner.sh: line 9: ".to_string());
            assert_eq!(origin_now(), "inner.sh: line 9: ");
        }
        assert_eq!(origin_now(), "outer.sh: line 1: ");
    }
}
