//! `emit` — fire a user event from a script.
//!
//! **The Lua side of this already existed.** `oslo.on.user(name, f)` attaches and
//! `oslo.on.emit(name, payload)` fires, which is what lets two plugins agree on a moment oslo never
//! anticipated. What had no door was the shell: a script could not say "the deploy finished" to
//! anything listening, so a plugin and a script could only meet through the filesystem.
//!
//! ```sh
//! emit deploy-done "$version"
//! ```
//!
//! Same storage, same order, same handlers as the Lua call — two doors onto one list, so an event
//! cannot mean different things depending on who fired it.

use crate::env::Environment;
use oslo_base::error::Result;
use oslo_base::value::Value;

/// `emit NAME [ARG...]`.
///
/// The status is **0 when a handler heard it and 1 when none did**, which is the only way a script
/// can tell "nobody is listening" from "it was delivered". A handler that raises is reported and
/// stepped over, and still counts as having heard: whether a listener worked is its own business,
/// not the emitter's.
pub fn builtin_emit(env: &mut Environment, args: &[String]) -> Result<i32> {
    let _ = env;
    let Some(name) = args.get(1) else {
        crate::env::complain(
            args,
            "emit",
            "emit: needs the name of an event",
            "no event named",
            Some("`emit deploy-done` fires the handlers attached by `oslo.on.user`"),
        );
        return Ok(2);
    };
    // A name with a space in it can be attached to from Lua but never emitted from a shell word,
    // so it would be a name that looks like it works and cannot.
    if name.is_empty() || name.contains(char::is_whitespace) {
        crate::env::complain(
            args,
            name,
            &format!("emit: {name:?}: not a usable event name"),
            "an event name is one word",
            Some("names are matched exactly; `deploy-done` rather than `deploy done`"),
        );
        return Ok(2);
    }

    // The operands as a Lua sequence, so a handler reads `payload[1]` — and a single argument is
    // still a one-element list rather than a bare string, because a handler that has to test which
    // it was received is a handler nobody wants to write.
    let payload = args.get(2..).unwrap_or_default();
    let mut table = oslo_base::value::Table::default();
    for (at, argument) in payload.iter().enumerate() {
        table.set(Value::int(at as i64 + 1), Value::str(argument));
    }
    let heard = oslo_base::hooks::emit(
        name,
        Value::Table(std::rc::Rc::new(std::cell::RefCell::new(table))),
    );
    Ok(i32::from(heard == 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emit(args: &[&str]) -> i32 {
        let mut env = Environment::new();
        let argv: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        builtin_emit(&mut env, &argv).expect("emit must not unwind")
    }

    /// **Nobody listening is 1, not 0.** It is the only way a script can tell that its event went
    /// nowhere, and with no interpreter attached that is every event.
    #[test]
    fn an_event_nobody_hears_reports_it() {
        assert_eq!(emit(&["emit", "deploy-done"]), 1);
        assert_eq!(emit(&["emit", "deploy-done", "1.2.3"]), 1);
    }

    /// A name that could be attached to but never emitted is worse than a refusal.
    #[test]
    fn a_name_that_could_not_be_emitted_is_refused() {
        assert_eq!(emit(&["emit"]), 2);
        assert_eq!(emit(&["emit", ""]), 2);
        assert_eq!(emit(&["emit", "two words"]), 2);
    }
}
