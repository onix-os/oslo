//! `oslo.proc` and `oslo.job` — signals and the job table.
//!
//! A shell already owns both; the point here is that Lua reaches the *same* ones. `oslo.job.list()`
//! shows the jobs `jobs` shows, and `job:foreground()` is what `fg %1` does — not a parallel job
//! system that agrees with the shell's until it does not.
//!
//! Signals are named, never numbered: `oslo.proc.kill(pid, "TERM")`. The numbers differ between
//! architectures, and a script that hard-codes 9 is a script that works until someone runs it
//! somewhere else.

use super::util::{failed, int, list, ok, opt_text, put, record, text};
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use oslo_base::value::LuaError;
use oslo_base::value::{Table, Value};
use oslo_shell::exec::job::{JobState, with_jobs};

/// Build the `oslo.proc` table.
pub fn build_proc() -> Value {
    let mut proc = Table::new();

    // oslo.proc.sleep(seconds) -> nil, after waiting
    //
    // **The stock `os` table has `clock`, `date`, `time` and `difftime`, and no sleep.** A script
    // that wanted to wait shelled out to `/bin/sleep`, which costs a fork and cannot portably take
    // a fraction. Seconds as a float, because waiting for less than one is the common case.
    super::util::put(&mut proc, "sleep", |_, args| {
        let seconds = match args.first() {
            Some(Value::Number(n)) => n.as_float(),
            other => {
                return Err(oslo_base::value::LuaError::new(format!(
                    "oslo.proc.sleep: argument #1 must be seconds, got {}",
                    other.map_or("no value", Value::type_name)
                )));
            }
        };
        // **The shared clamp, and it is not politeness.** `Duration::from_secs_f64` *panics* on a
        // number a `Duration` cannot hold, so `oslo.proc.sleep(1e30)` from a config would abort the
        // shell uncatchably — the same way `oslo.after` once did, which is why the clamp exists.
        std::thread::sleep(super::util::seconds_to_duration(seconds));
        super::util::ok(Value::Nil)
    });

    // What a process *is*, read from `/proc` rather than parsed out of `ps`. See
    // [`super::procinfo`].
    super::procinfo::install(&mut proc);

    // oslo.proc.kill(pid, signal) -> true, or nil + message
    put(&mut proc, "kill", |_, args| {
        let pid = int(&args, 1, "oslo.proc.kill")?;
        let name = opt_text(&args, 2, "oslo.proc.kill")?.unwrap_or_else(|| "TERM".to_string());
        let signal = signal_named(&name)?;
        match nix::sys::signal::kill(Pid::from_raw(pid as i32), signal) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed(&format!("kill {pid}"), e),
        }
    });

    // oslo.proc.alive(pid) -> boolean
    //
    // Signal 0 is the POSIX way to ask: it performs the permission check and finds the process
    // without delivering anything.
    put(&mut proc, "alive", |_, args| {
        let pid = int(&args, 1, "oslo.proc.alive")?;
        ok(Value::Bool(
            nix::sys::signal::kill(Pid::from_raw(pid as i32), None).is_ok(),
        ))
    });

    put(&mut proc, "signals", |_, _| {
        ok(list(Signal::iterator().map(|s| {
            // Without the `SIG` prefix, which is how `kill -TERM` and `trap … TERM` spell it.
            Value::str(s.as_str().trim_start_matches("SIG"))
        })))
    });

    put(&mut proc, "signal_number", |_, args| {
        let name = text(&args, 1, "oslo.proc.signal_number")?;
        ok(Value::int(signal_named(&name)? as i64))
    });

    // oslo.proc.parse(line) -> { { link, kind, argv, redirects }, … }, or nil
    //
    // **The shell's own parser, so a config stops splitting on whitespace.** A builtin or a hook
    // handed a command line reaches for `gmatch("%S+")`, which is wrong for `cat 'a b'`, wrong for
    // `echo $HOME`, and wrong for `a | b` — the exact three cases the parser's own header names.
    //
    // Pure, so it works inside a registered builtin where `oslo.run` raises. That is most of the
    // reason to have it: the surface that cannot run a command is the one that most needs to
    // understand one.
    //
    // `redirects` is a **count**, not a list — the same shape `pre-cmd` already receives, because
    // this is that payload with a door on it. Changing it would break a documented hook.
    put(&mut proc, "parse", |_, args| {
        let line = text(&args, 1, "oslo.proc.parse")?;
        // A line that does not parse answers nil rather than raising: the shell is about to report
        // the syntax error far better than a caller could. See `crate::lua::parsed`.
        ok(crate::lua::parsed::commands_of(&line).unwrap_or(Value::Nil))
    });

    Value::table(proc)
}

/// A signal by name, with or without the `SIG` prefix, in any case.
fn signal_named(name: &str) -> Result<Signal, LuaError> {
    let wanted = name.trim().to_ascii_uppercase();
    let bare = wanted.strip_prefix("SIG").unwrap_or(&wanted);
    Signal::iterator()
        .find(|s| s.as_str().trim_start_matches("SIG") == bare)
        .ok_or_else(|| {
            // Named rather than guessed at. A typo silently becoming SIGTERM is how a script that
            // meant to check on a process kills it.
            LuaError::new(format!("oslo.proc: no signal called '{name}'"))
        })
}

/// Build the `oslo.job` table.
pub fn build_job() -> Value {
    let mut job = Table::new();

    // oslo.job.watcher() -> what repeated Ctrl-C is set up to do, and whether anything is doing it
    //
    // **Reports the setting *and* whether it is live**, because those come apart in the one way
    // that matters: a config can ask for escalation in a shell that is not interactive, or has no
    // job control, and there the watcher is never forked and Ctrl-C means what it always did. A
    // caller reading only the setting would report a feature that is not running.
    put(&mut job, "watcher", |_, _| {
        let escape = oslo_ui::settings::current().misc.interrupt_escape;
        ok(record(vec![
            ("after", Value::int(escape.after as i64)),
            ("action", Value::str(escape.action.name())),
            ("notify", Value::Bool(escape.notify)),
            (
                "running",
                Value::Bool(escape.after > 0 && oslo_shell::exec::job::watcher_started()),
            ),
        ]))
    });

    // oslo.job.list() -> the same jobs `jobs` prints, as records
    put(&mut job, "list", |_, _| {
        let jobs = with_jobs(|table| {
            table
                .jobs()
                .iter()
                .map(|j| {
                    record(vec![
                        ("id", Value::int(j.id as i64)),
                        ("pgid", Value::int(j.pgid.as_raw() as i64)),
                        ("command", Value::str(&j.command)),
                        ("state", Value::str(state_name(&j.state))),
                        // Present only once the job has finished, which is what tells "done"
                        // apart from "done, and it failed".
                        (
                            "status",
                            match &j.state {
                                JobState::Completed(status) => Value::int(*status as i64),
                                _ => Value::Nil,
                            },
                        ),
                    ])
                })
                .collect::<Vec<_>>()
        });
        ok(list(jobs))
    });

    // oslo.job.foreground(id) -> the status it finished with, or nil + message
    put(&mut job, "foreground", |_, args| {
        let id = int(&args, 1, "oslo.job.foreground")? as usize;
        match oslo_shell::exec::job::foreground_job(id) {
            Some(status) => ok(Value::int(status as i64)),
            None => failed("oslo.job.foreground", format!("no job {id}")),
        }
    });

    put(&mut job, "background", |_, args| {
        let id = int(&args, 1, "oslo.job.background")? as usize;
        if oslo_shell::exec::job::continue_in_background(id) {
            return ok(Value::Bool(true));
        }
        failed("oslo.job.background", format!("no job {id}"))
    });

    // oslo.job.signal(id, name) — to the whole process *group*, which is what makes a signal
    // reach every stage of a pipeline rather than only the one the shell happened to record.
    put(&mut job, "signal", |_, args| {
        let id = int(&args, 1, "oslo.job.signal")? as usize;
        let name = opt_text(&args, 2, "oslo.job.signal")?.unwrap_or_else(|| "TERM".to_string());
        let signal = signal_named(&name)?;
        let Some(pgid) = with_jobs(|table| table.get(id).map(|j| j.pgid)) else {
            return failed("oslo.job.signal", format!("no job {id}"));
        };
        match nix::sys::signal::killpg(pgid, signal) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed(&format!("oslo.job.signal {id}"), e),
        }
    });

    Value::table(job)
}

fn state_name(state: &JobState) -> &'static str {
    match state {
        JobState::Running => "running",
        JobState::Stopped => "stopped",
        JobState::Completed(_) => "done",
    }
}
