//! Handing the transcript to another program.
//!
//! The line editor draws what a finished line leaves behind — see [`oslo_ui::transcript`] — and it
//! may be told to hand that job over: `oslo.transcript.command` names something that prints the
//! block. Running programs is this crate's business, not the editor's, so the editor asks a
//! function and this installs it. The same shape [`oslo_base::background`] uses for its servicer,
//! and for the same reason.
//!
//! # Not the prompt's cache
//!
//! [`crate::lua::api::external::render`] would be the obvious thing to reuse and is the wrong
//! thing: it is keyed on the *unsubstituted* argv, so every command in a session shares one key,
//! and its guard would hand back the frame drawn for the command before this one. A transcript is
//! new every time by definition. So this runs the tool directly.
//!
//! # It is on the path of every command
//!
//! Between pressing Enter and the command starting. That is why the deadline is short and why
//! there is no `async`: a frame that arrives after the command has already printed is not a frame,
//! and there is nothing sensible to show in the meantime. A tool that overruns is killed and the
//! rule is drawn instead, which is what `None` means to the caller.

use oslo_base::value::Value;
use std::time::Duration;

/// Read `oslo.transcript.command` and, if it names one, install the renderer.
///
/// Called once at startup, after the config has run. A config that names nothing installs nothing,
/// and the editor falls back to `oslo.transcript.rule` — which is the whole of the common case.
pub fn install(oslo: &Value) {
    let Some(spec) = spec_of(oslo) else {
        return;
    };
    oslo_ui::transcript::install(move |command| {
        let args: Vec<String> = spec
            .args
            .iter()
            .map(|arg| arg.replace("$command", command))
            .collect();
        crate::lua::api::external::run(&spec.command, &args, spec.timeout)
    });
}

/// What a config asked for, if it asked for anything.
struct Spec {
    command: String,
    args: Vec<String>,
    timeout: Duration,
}

fn spec_of(oslo: &Value) -> Option<Spec> {
    let Value::Table(oslo) = oslo else {
        return None;
    };
    let Value::Table(transcript) = oslo.borrow().get_str("transcript") else {
        return None;
    };
    let Value::Table(spec) = transcript.borrow().get_str("command") else {
        return None;
    };
    let spec = spec.borrow();
    let Value::Str(command) = spec.get_str("command") else {
        return None;
    };
    let mut args = Vec::new();
    if let Value::Table(list) = spec.get_str("args") {
        let list = list.borrow();
        for i in 1..=list.length() {
            match list.get(&Value::int(i)) {
                Value::Str(word) => args.push(word.to_string()),
                Value::Number(n) => args.push(n.to_string()),
                _ => {}
            }
        }
    }
    let timeout = match spec.get_str("timeout_ms") {
        Value::Number(n) => n.as_int().unwrap_or(20).max(1) as u64,
        // Short, because this is between Enter and the command running. A frame is worth a few
        // milliseconds and not more; past that the rule is the better answer.
        _ => 20,
    };
    Some(Spec {
        command: command.to_string(),
        args,
        timeout: Duration::from_millis(timeout),
    })
}

#[cfg(test)]
#[path = "transcript/tests.rs"]
mod tests;
