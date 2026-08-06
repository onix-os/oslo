//! Telling you a long command has finished.
//!
//! Split from the REPL because it is a self-contained decision — is this worth interrupting for,
//! and how is the interruption delivered — with three settings behind it and no state of its own.

use oslo::error::ShellError;

/// A notification for a command that ran long enough to be worth one, or nothing.
///
/// The threshold is `oslo.notify.after`, in seconds, and `0` turns it off. There is deliberately no
/// check for whether the terminal is focused: reporting focus needs a mode the shell would have to
/// enable and then read replies for, and getting that wrong costs stray characters at the prompt —
/// a worse failure than a notification you did not need.
pub(super) fn slow_command_notice(
    text: &str,
    elapsed: std::time::Duration,
    result: &Result<i32, ShellError>,
) -> String {
    let after = oslo::interactive::settings::current().notify.after;
    if after == 0 || elapsed.as_secs() < after || !oslo::feature::on(oslo::feature::at::NOTIFY) {
        return String::new();
    }
    // The config gets first refusal. **This one fires from the read loop with nothing locked**, so
    // a handler here may use the whole `oslo.*` API — unlike `chain`, `job` and `time`, which fire
    // from inside a builtin or the executor. See `oslo::interactive::report`.
    if drawn_by_config(text, elapsed, result) {
        return String::new();
    }
    let status = result.as_ref().copied().unwrap_or(1);
    let outcome = if status == 0 {
        "finished".to_string()
    } else {
        format!("failed ({status})")
    };
    let took = oslo::interactive::prompt::notable_duration(elapsed)
        .unwrap_or_else(|| format!("{}s", elapsed.as_secs()));
    // The first word, as the title bar gets: a notification is narrow too.
    let what = text.split_whitespace().next().unwrap_or("command");
    let body = format!("{outcome} after {took}");
    let words = oslo::interactive::settings::current().notify_text;
    let fill = |template: &str| {
        template
            .replace("{cmd}", text)
            .replace("{status}", &status.to_string())
            .replace("{duration}", &took)
    };
    let title = words.title.as_deref().map(fill);
    let title = title.as_deref().unwrap_or(what);

    // A command of your own instead of the escape. Run detached and its output discarded: this is
    // a side effect between a command finishing and the next prompt, and a `notify-send` that
    // printed a warning there would corrupt the row the prompt is about to be drawn on.
    if let Some(command) = &words.command {
        let filled = fill(command)
            .replace("{title}", title)
            .replace("{body}", &body);
        let _ = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&filled)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        return String::new();
    }
    oslo::interactive::marks::notify(title, &body)
}

/// Whether an `on-report` handler dealt with the slow-command notice instead.
fn drawn_by_config(
    text: &str,
    elapsed: std::time::Duration,
    result: &Result<i32, ShellError>,
) -> bool {
    use oslo::interactive::report::{self, int, text as string};
    if !report::watched() {
        return false;
    }
    let status = result.as_ref().copied().unwrap_or(1);
    report::handled(
        "slow",
        vec![
            ("text", string(text)),
            ("duration_ms", int(elapsed.as_millis() as i64)),
            ("status", int(i64::from(status))),
            ("ok", oslo::lua::eval::value::Value::Bool(status == 0)),
        ],
    )
}
