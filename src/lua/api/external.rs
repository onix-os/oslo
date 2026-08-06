//! A prompt produced by another program — starship, hexe, anything that writes a prompt to stdout.
//!
//! ```lua
//! oslo.prompt.left = {
//!   command = "starship",
//!   args = { "prompt", "--status", "$status", "--cmd-duration", "$duration_ms" },
//!   timeout_ms = 200,
//!   async = true,
//! }
//! ```
//!
//! The `$name` arguments are filled from the same context a segment's `render(ctx)` receives, so
//! the tool is told the exit status and how long the last command took without the config having to
//! plumb them through the environment.
//!
//! **Two protections, because a prompt is on the critical path of every keystroke.**
//!
//! `timeout_ms` bounds how long the shell will wait. A tool that hangs — a network mount, a git
//! repository the size of a planet — cannot hang the shell; the deadline passes and the last
//! prompt it produced is used instead.
//!
//! `async` goes further: the previous output is used *immediately* and the tool runs behind the
//! prompt, so its cost is never on the path at all. The first prompt of a session has nothing to
//! reuse and waits for one run, which is the only time it can be seen.

use crate::lua::context::Context;
use crate::lua::eval::value::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// What a config asked for.
pub struct Spec {
    pub command: String,
    pub args: Vec<String>,
    pub timeout: Duration,
    pub asynchronous: bool,
}

/// Read the external-prompt form out of a Lua table, or `None` if it is not one.
pub fn spec_of(value: &Value) -> Option<Spec> {
    let Value::Table(t) = value else {
        return None;
    };
    let t = t.borrow();
    let Value::Str(command) = t.get(&Value::str("command")) else {
        return None;
    };
    let mut args = Vec::new();
    if let Value::Table(list) = t.get(&Value::str("args")) {
        let list = list.borrow();
        for i in 1..=list.length() {
            match list.get(&Value::int(i)) {
                Value::Str(s) => args.push(s.to_string()),
                Value::Number(n) => args.push(n.to_string()),
                _ => {}
            }
        }
    }
    let timeout = match t.get(&Value::str("timeout_ms")) {
        Value::Number(n) => n.as_int().unwrap_or(200).max(1) as u64,
        // Long enough for a tool doing real work, short enough that a hung one is noticed as a
        // pause rather than as a dead shell.
        _ => 200,
    };
    Some(Spec {
        command: command.to_string(),
        args,
        timeout: Duration::from_millis(timeout),
        asynchronous: t.get(&Value::str("async")).truthy(),
    })
}

/// Substitute `$name` in an argument from the context.
///
/// **Every field a prompt segment can render is substitutable here**, and that is the rule rather
/// than a coincidence: an external prompt is oslo describing itself to a program that cannot look
/// inside, so anything a native segment may use it must be able to say out loud. `$vimode`, `$user`
/// and `$host` were in [`Context`] and missing from this list, which made them reachable from a
/// Lua segment and unreachable from starship, hexe or anything else run as a command.
///
/// An absent optional becomes the **empty string**, not the word `none`: the receiving program is
/// being told "no answer", and every argument parser already knows what an empty value means.
fn fill(arg: &str, ctx: &Context) -> String {
    let mut out = arg.to_string();
    for (name, value) in [
        ("$status", ctx.status.to_string()),
        ("$duration_ms", ctx.duration_ms.unwrap_or(0).to_string()),
        ("$cwd", ctx.cwd.clone()),
        ("$cols", ctx.cols.to_string()),
        ("$jobs", ctx.jobs.to_string()),
        ("$language", ctx.language.clone()),
        ("$branch", ctx.branch.clone().unwrap_or_default()),
        ("$vimode", ctx.vimode.clone().unwrap_or_default()),
        ("$user", ctx.user.clone()),
        ("$host", ctx.host.clone()),
    ] {
        out = out.replace(name, &value);
    }
    out
}

/// The last output each command produced, so a slow or async run has something to show.
fn cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remembered(key: &str) -> Option<String> {
    cache().lock().ok()?.get(key).cloned()
}

fn remember(key: &str, value: String) {
    if let Ok(mut c) = cache().lock() {
        c.insert(key.to_string(), value);
    }
}

/// Run the tool and return what it printed.
///
/// `None` when it could not be run at all, so the caller falls back to a prompt of its own rather
/// than drawing an empty line.
pub fn render(spec: &Spec, ctx: &Context) -> Option<String> {
    let args: Vec<String> = spec.args.iter().map(|a| fill(a, ctx)).collect();
    let key = format!("{} {}", spec.command, args.join(" "));

    if spec.asynchronous {
        // Whatever it said last time, now; the fresh run lands in the cache for the next prompt.
        let previous = remembered(&key);
        spawn(spec.command.clone(), args, key);
        return previous;
    }

    match run(&spec.command, &args, spec.timeout) {
        Some(out) => {
            remember(&key, out.clone());
            Some(out)
        }
        // It overran or failed. The last good answer beats a blank prompt, and beats waiting.
        None => remembered(&key),
    }
}

/// Run it in the background, keeping the result for the next prompt.
fn spawn(command: String, args: Vec<String>, key: String) {
    std::thread::spawn(move || {
        // No deadline here: nothing is waiting for it, and a hung tool costs one thread rather
        // than the prompt. The next run replaces whatever this one eventually says.
        if let Some(out) = run(&command, &args, Duration::from_secs(10)) {
            remember(&key, out);
        }
    });
}

/// Run a command with a deadline, returning its stdout with the trailing newline trimmed.
fn run(command: &str, args: &[String], timeout: Duration) -> Option<String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Left alone on purpose: a tool's complaints belong on the terminal where the user can see
        // them, not folded into the prompt.
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(None) => {
                // Overran. Killed rather than left behind: one hung prompt tool per keystroke
                // would otherwise become hundreds of processes in a session.
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            // `ECHILD`: something reaped the child before this loop could — a `SIGCHLD`
            // disposition of `SIG_IGN` does exactly that, and a shell is a program likely to have
            // one. It means the command *finished*, not that it failed, so fall through and read
            // what it wrote. Returning here made every run produce nothing whenever such a handler
            // was installed, which is how the test found it.
            Err(_) => break,
        }
    }

    // Read the pipe *after* the wait loop, and not with `wait_with_output`: the loop above has
    // already reaped the child, so `wait_with_output` fails and every run would return nothing.
    // The test caught exactly that.
    use std::io::Read;
    let mut text = String::new();
    child.stdout.take()?.read_to_string(&mut text).ok()?;
    Some(text.trim_end_matches('\n').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            status: 3,
            duration_ms: Some(1500),
            cwd: "/tmp/x".to_string(),
            cols: 100,
            language: "lua".to_string(),
            ..Context::default()
        }
    }

    /// The tool is told what the shell knows, without the config plumbing it through the
    /// environment by hand.
    #[test]
    fn arguments_are_filled_from_the_context() {
        assert_eq!(fill("--status=$status", &ctx()), "--status=3");
        assert_eq!(
            fill("--cmd-duration=$duration_ms", &ctx()),
            "--cmd-duration=1500"
        );
        assert_eq!(
            fill("--terminal-width=$cols", &ctx()),
            "--terminal-width=100"
        );
        // A name that is not a placeholder is left exactly as written.
        assert_eq!(fill("--keep-$this", &ctx()), "--keep-$this");
    }

    /// **Every renderable field can be named.** A field that a Lua segment can read but an
    /// external prompt cannot ask for is a field that works in one prompt and silently vanishes in
    /// the other — which is how `$vimode` came to exist on `Context` and be unreachable from
    /// starship or hexe. If a field is added to `Context`, it is added here too.
    #[test]
    fn every_context_field_a_prompt_can_render_is_substitutable() {
        let mut facts = ctx();
        facts.vimode = Some("normal".to_string());
        facts.user = "ada".to_string();
        facts.host = "lovelace".to_string();
        facts.language = "lua".to_string();
        facts.branch = Some("main".to_string());

        assert_eq!(fill("$vimode", &facts), "normal");
        assert_eq!(fill("$user@$host", &facts), "ada@lovelace");
        assert_eq!(fill("$language", &facts), "lua");
        assert_eq!(fill("$branch", &facts), "main");

        // An absent optional is the empty string, so `--vimode=` reaches the program as "no
        // answer" rather than as the literal word `none` it would then have to special-case.
        facts.vimode = None;
        facts.branch = None;
        assert_eq!(fill("--vimode=$vimode", &facts), "--vimode=");
        assert_eq!(fill("--branch=$branch", &facts), "--branch=");
    }

    /// A tool that never finishes must not become a shell that never prompts.
    #[test]
    fn a_command_that_overruns_is_killed_and_reported_as_nothing() {
        let started = std::time::Instant::now();
        let sleep = ["/bin/sleep", "/usr/bin/sleep"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
            .expect("a system sleep");
        let out = run(sleep, &["10".to_string()], Duration::from_millis(60));
        assert!(out.is_none(), "an overrun produces nothing, not a hang");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the deadline was not honoured: waited {:?}",
            started.elapsed()
        );
    }

    /// What it printed, without the newline every tool ends with.
    #[test]
    fn output_is_taken_verbatim_less_the_trailing_newline() {
        // An absolute path, not `echo`: other tests in this binary mutate the process-wide
        // `$PATH`, and a bare name would make this test depend on whichever of them ran first.
        let echo = ["/bin/echo", "/usr/bin/echo"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
            .expect("a system echo");
        let out = run(echo, &["hi".to_string()], Duration::from_secs(30));
        assert_eq!(out.as_deref(), Some("hi"));
    }
}
