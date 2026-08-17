//! `oslo.proc.info` and `oslo.proc.children` — what a process is, without parsing `ps`.
//!
//! ```lua
//! local me = oslo.proc.info(oslo.proc.pid())
//! print(me.name, me.state, me.rss)          -- "oslo"  "running"  18874368
//!
//! for _, child in ipairs(oslo.proc.children(oslo.proc.pid())) do
//!   print(child.pid, child.command)
//! end
//! ```
//!
//! # Why not `ps`
//!
//! `ps` is the canonical thing to shell out to and the canonical thing to get wrong. Its columns are
//! padded to a width that depends on the values in them, `COMMAND` contains spaces so it cannot be
//! the *n*th field, and a process whose name has a space or a newline in it — which is allowed, and
//! which anything hostile will have — breaks every `awk '{print $2}'` ever written. The kernel
//! already publishes all of this as fields; this reads those.
//!
//! It is also the difference between a fact and a rendering: `rss` here is bytes, so
//! `if p.rss > 1e9` works. `ps` would have said `1.2g`.
//!
//! # What is missing, and why
//!
//! **`exe` and `cwd` may be `nil` for a process that is not yours.** Both are symlinks in `/proc`
//! that only the owner and root may read, so the answer is genuinely unavailable rather than
//! empty — and `nil` says that where `""` would look like a process with no executable.

use super::util::{failed_path, list, ok, put, record};
use oslo_base::value::{LuaError, Table, Value};
use std::fs;
use std::path::PathBuf;

/// Add the process-inspection calls to `oslo.proc`.
pub fn install(proc: &mut Table) {
    // oslo.proc.info(pid) -> a table of facts, or nil + message when there is no such process
    put(proc, "info", |_, args| {
        let pid = pid_of(args.first(), "oslo.proc.info")?;
        match read(pid) {
            Some(info) => ok(info),
            None => failed_path(
                &format!("/proc/{pid}"),
                &std::io::Error::from(std::io::ErrorKind::NotFound),
            ),
        }
    });

    // oslo.proc.children(pid) -> the processes whose parent it is, as info tables
    //
    // **A scan of `/proc`, because the kernel publishes the edge the other way round.** Each
    // process records its parent; nothing records a list of children, so finding them means
    // looking at everybody. That is a few hundred small reads and is why this is a call rather
    // than a field on `info`.
    put(proc, "children", |_, args| {
        let parent = pid_of(args.first(), "oslo.proc.children")?;
        let mut found: Vec<(i64, Value)> = Vec::new();
        let Ok(entries) = fs::read_dir("/proc") else {
            return ok(list([]));
        };
        for entry in entries.flatten() {
            let Some(pid) = entry.file_name().to_string_lossy().parse::<i64>().ok() else {
                continue;
            };
            // A process that exits between the listing and the read is not an error — it is the
            // ordinary case on a busy machine, and skipping it is the only right answer.
            if let Some(info) = read(pid)
                && parent_of(&info) == Some(parent)
            {
                found.push((pid, info));
            }
        }
        found.sort_by_key(|(pid, _)| *pid);
        ok(list(found.into_iter().map(|(_, info)| info)))
    });
}

/// The `ppid` field of an info table, for filtering.
fn parent_of(info: &Value) -> Option<i64> {
    let Value::Table(table) = info else {
        return None;
    };
    let ppid = table.borrow().get_str("ppid");
    ppid.as_number()?.as_int()
}

/// Argument 1 as a process id.
fn pid_of(value: Option<&Value>, function: &str) -> Result<i64, LuaError> {
    match value.and_then(Value::as_number).and_then(|n| n.as_int()) {
        Some(pid) if pid > 0 => Ok(pid),
        _ => Err(LuaError::new(format!(
            "{function}: argument #1 must be a process id"
        ))),
    }
}

/// Everything `/proc/<pid>` will say, or `None` when there is no such process.
fn read(pid: i64) -> Option<Value> {
    let at = PathBuf::from(format!("/proc/{pid}"));
    let status = fs::read_to_string(at.join("status")).ok()?;

    let argv = argv_of(&at);
    Some(record(vec![
        ("pid", Value::int(pid)),
        // From `status` rather than from `stat`, whose second field is the name in parentheses and
        // may itself contain a `)` — a process can be called `foo) bar`, and anything splitting
        // `stat` on whitespace reads the wrong field for the rest of the line when it is.
        ("name", text_field(&status, "Name")),
        ("state", state_of(&status)),
        ("ppid", number_field(&status, "PPid")),
        ("threads", number_field(&status, "Threads")),
        // Bytes, not the kibibytes `/proc` reports, so a threshold is written in the unit anybody
        // would write it in.
        ("rss", bytes_field(&status, "VmRSS")),
        ("size", bytes_field(&status, "VmSize")),
        ("uid", first_number(&status, "Uid")),
        (
            "command",
            match argv.is_empty() {
                // A kernel thread has an empty `cmdline`; its name in brackets is what `ps` shows
                // and is the only thing there is to show.
                true => match text_field(&status, "Name") {
                    Value::Str(name) => Value::str(format!("[{name}]")),
                    other => other,
                },
                false => Value::str(argv.join(" ")),
            },
        ),
        ("argv", list(argv.iter().map(Value::str))),
        ("exe", link(&at.join("exe"))),
        ("cwd", link(&at.join("cwd"))),
    ]))
}

/// `/proc/<pid>/cmdline`, which is NUL-separated rather than space-separated.
///
/// **Which is the whole reason this is worth reading directly.** The arguments are exactly as they
/// were passed, so one containing a space is one argument here and two in anything that re-splits
/// `ps` output.
fn argv_of(at: &std::path::Path) -> Vec<String> {
    let Ok(raw) = fs::read(at.join("cmdline")) else {
        return Vec::new();
    };
    raw.split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

/// A `/proc` symlink, or `nil` when it is not readable.
fn link(path: &std::path::Path) -> Value {
    match fs::read_link(path) {
        Ok(target) => Value::str(target.to_string_lossy()),
        // `EACCES` for somebody else's process, which is a real answer and not an empty one.
        Err(_) => Value::Nil,
    }
}

/// The state letter as a word.
///
/// Named rather than passed through, because `"R"` means nothing at a call site and every reader
/// has to go and look it up. The letters are `proc(5)`'s.
fn state_of(status: &str) -> Value {
    let Value::Str(raw) = text_field(status, "State") else {
        return Value::Nil;
    };
    let name = match raw.chars().next() {
        Some('R') => "running",
        Some('S') => "sleeping",
        // Uninterruptible sleep: waiting on I/O, and the state that makes a process unkillable.
        Some('D') => "waiting",
        Some('Z') => "zombie",
        Some('T') => "stopped",
        Some('t') => "traced",
        Some('X') | Some('x') => "dead",
        Some('I') => "idle",
        _ => "other",
    };
    Value::str(name)
}

/// A `Name:\tvalue` line's value.
fn text_field(status: &str, name: &str) -> Value {
    match field(status, name) {
        Some(value) => Value::str(value),
        None => Value::Nil,
    }
}

/// The same, as a number.
fn number_field(status: &str, name: &str) -> Value {
    match field(status, name).and_then(|value| value.split_whitespace().next()?.parse::<i64>().ok())
    {
        Some(n) => Value::int(n),
        None => Value::Nil,
    }
}

/// A `VmRSS:  1234 kB` line, as bytes.
fn bytes_field(status: &str, name: &str) -> Value {
    match number_field(status, name) {
        Value::Number(n) => match n.as_int() {
            Some(kib) => Value::int(kib * 1024),
            None => Value::Nil,
        },
        _ => Value::Nil,
    }
}

/// The first of the four ids on a `Uid:` line — the real one, which is who owns the process.
fn first_number(status: &str, name: &str) -> Value {
    number_field(status, name)
}

/// The text after `name:` on its line.
fn field<'a>(status: &'a str, name: &str) -> Option<&'a str> {
    status.lines().find_map(|line| {
        let (field, value) = line.split_once(':')?;
        (field == name).then(|| value.trim())
    })
}

#[cfg(test)]
#[path = "procinfo/tests.rs"]
mod tests;
