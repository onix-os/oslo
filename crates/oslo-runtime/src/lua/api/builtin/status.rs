//! Turning what a builtin returned into an exit status.
//!
//! Moved out of `engine.rs` verbatim when the builtin contract grew a table form: the engine owns
//! the Lua state, and how a *builtin's* answer is read belongs with the rest of the builtin. It was
//! also the cheapest way to buy back the lines the new contract needed — `engine.rs` was fourteen
//! from the limit.

use oslo_base::value::Value;

/// Turn whatever a Lua builtin returned into an exit status.
///
/// Modelled on how a shell reads a command's result rather than on Lua's own truthiness: no
/// return value at all is success (the common case — a builtin that just printed something),
/// `false` is failure, and a number is the status the script asked for.
pub(crate) fn from_lua(value: Option<&Value>) -> i32 {
    match value {
        None | Some(Value::Nil) | Some(Value::Bool(true)) => 0,
        Some(Value::Bool(false)) => 1,
        Some(Value::Number(n)) => n.as_float() as i32,
        Some(Value::Str(s)) => s.parse().unwrap_or(0),
        Some(_) => 0,
    }
}
