//! What `oslo.sys` can say about the machine, read from `/proc` rather than from `uname` and `free`.
//!
//! ```lua
//! oslo.sys.kernel()        -- "7.0.0-29-generic"
//! oslo.sys.arch()          -- "x86_64"
//! oslo.sys.cpus()          -- 24
//! oslo.sys.uptime()        -- 257519.31, in seconds
//! oslo.sys.loadavg()       -- { 5.39, 3.23, 2.59 }
//! oslo.sys.memory()        -- { total = …, available = …, free = …, swap_total = …, swap_free = … }
//! ```
//!
//! # Why these belong in the shell rather than in a command
//!
//! They are what a prompt segment and a status line ask for, and they are asked *often* — the whole
//! reason `oslo.spawn` exists is that a prompt cannot afford a process per draw. `free | awk` is
//! three processes and a parse; this is one small read of a file the kernel keeps in memory.
//!
//! **Numbers, not renderings.** `memory().available` is bytes, not `39G`, for the same reason the
//! structured pipeline hands over a `Size` as bytes: a caller that wants to compare needs a number,
//! and one that wants to show it has `oslo.ui.format`. A function answering `"39G"` makes every
//! comparison a string comparison and every threshold wrong.

use super::util::{list, ok, put, record};
use oslo_base::value::{Table, Value};

/// Add the machine facts to the `oslo.sys` table.
pub fn install(sys: &mut Table) {
    // oslo.sys.kernel() -> the release string, or nil
    put(sys, "kernel", |_, _| {
        ok(first_line("/proc/sys/kernel/osrelease"))
    });

    // oslo.sys.arch() -> "x86_64"
    //
    // **The binary's architecture, which is not always the machine's.** An x86_64 build running
    // under emulation reports `x86_64` while the hardware is something else — and that is the
    // honest answer to the question a script is actually asking, which is what will run here.
    put(sys, "arch", |_, _| ok(Value::str(std::env::consts::ARCH)));

    // oslo.sys.cpus() -> how many the shell may run on
    //
    // `available_parallelism`, not the line count of `/proc/cpuinfo`: it respects a cgroup quota
    // and a CPU affinity mask, so a shell in a container answers what the container has rather
    // than what the host does.
    put(sys, "cpus", |_, _| {
        ok(Value::int(
            std::thread::available_parallelism()
                .map(|n| n.get() as i64)
                .unwrap_or(1),
        ))
    });

    // oslo.sys.uptime() -> seconds since boot, with the fraction the kernel keeps
    put(sys, "uptime", |_, _| {
        ok(match read_at("/proc/uptime", 0) {
            Some(seconds) => Value::float(seconds),
            None => Value::Nil,
        })
    });

    // oslo.sys.loadavg() -> { 1 minute, 5 minutes, 15 minutes }
    //
    // A list rather than three fields, because they are one reading at three windows and every
    // caller either takes the first or shows all three.
    put(sys, "loadavg", |_, _| {
        let Ok(text) = std::fs::read_to_string("/proc/loadavg") else {
            return ok(Value::Nil);
        };
        let averages: Vec<Value> = text
            .split_whitespace()
            .take(3)
            .filter_map(|word| word.parse::<f64>().ok())
            .map(Value::float)
            .collect();
        ok(if averages.len() == 3 {
            list(averages)
        } else {
            Value::Nil
        })
    });

    // oslo.sys.memory() -> bytes, as a table, or nil
    put(sys, "memory", |_, _| {
        let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
            return ok(Value::Nil);
        };
        let field = |name: &str| match kilobytes(&text, name) {
            Some(bytes) => Value::int(bytes),
            None => Value::Nil,
        };
        ok(record(vec![
            ("total", field("MemTotal")),
            ("available", field("MemAvailable")),
            ("free", field("MemFree")),
            ("swap_total", field("SwapTotal")),
            ("swap_free", field("SwapFree")),
        ]))
    });
}

/// A `/proc/meminfo` field, as bytes.
///
/// The file reports kibibytes and says so on every line; the conversion is here so no caller has to
/// remember which unit it is in — the one thing about `/proc/meminfo` that everybody gets wrong once.
fn kilobytes(meminfo: &str, name: &str) -> Option<i64> {
    meminfo
        .lines()
        .find_map(|line| {
            let (field, rest) = line.split_once(':')?;
            if field != name {
                return None;
            }
            rest.split_whitespace().next()?.parse::<i64>().ok()
        })
        .map(|kib| kib * 1024)
}

/// Word `n` of a file, as a number.
fn read_at(path: &str, n: usize) -> Option<f64> {
    let text = std::fs::read_to_string(path).ok()?;
    text.split_whitespace().nth(n)?.parse().ok()
}

/// A file's first line, trimmed, or nil.
fn first_line(path: &str) -> Value {
    match std::fs::read_to_string(path) {
        Ok(text) => match text.lines().next() {
            Some(line) if !line.trim().is_empty() => Value::str(line.trim()),
            _ => Value::Nil,
        },
        Err(_) => Value::Nil,
    }
}

#[cfg(test)]
#[path = "machine/tests.rs"]
mod tests;
