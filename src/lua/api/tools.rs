//! Per-command output parsers: `sh.df()` and friends, answering rows instead of text.
//!
//! The shape these follow is written up in `docs/built-in-tools.md`. The short version: a tool
//! answers in text when a pipe asks and in a table when Lua asks, and the fields carry **values,
//! not renderings** — `free` is a byte count, `free_human` is `"4.2G"`. A config that wants to
//! compare needs the number; one that wants to draw wants the string; making each config derive
//! one from the other is how they end up disagreeing.
//!
//! These parse the external tool's output rather than reimplementing it. That is the honest
//! starting point: `df` on this machine knows about this machine's filesystems, and a reimplementation
//! would be a second source of truth to keep in step. What the parser buys is that the *caller*
//! never has to `awk` a column out of text whose layout shifts with the mount point's length.

use crate::lua::eval::value::{Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// A byte count as something readable: `4.2G`, `918M`, `512B`.
fn human(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else if value < 10.0 {
        format!("{value:.1}{}", UNITS[unit])
    } else {
        format!("{value:.0}{}", UNITS[unit])
    }
}

/// One filesystem, as `df -P` describes it.
#[derive(Debug, PartialEq, Eq)]
pub struct Filesystem {
    pub source: String,
    pub size: u64,
    pub used: u64,
    pub free: u64,
    pub capacity: u8,
    pub mount: String,
}

/// Parse `df -P` output.
///
/// `-P` is the POSIX format, which guarantees one filesystem per line and a fixed column order —
/// without it the source is wrapped onto its own line when it is long, and every field shifts.
///
/// **The mount point is taken as the rest of the line, not as a field.** It is the last column and
/// it may contain spaces, so splitting on whitespace and taking element six loses `/mnt/my disk`.
/// This is the bug that makes `df | awk '{print $6}'` wrong on exactly the machines where it
/// matters, and not having it is most of the reason this parser exists.
pub fn parse_df(output: &str) -> Vec<Filesystem> {
    let mut out = Vec::new();
    for line in output.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let Some(source) = fields.next() else {
            continue;
        };
        let numbers: Vec<&str> = fields.by_ref().take(4).collect();
        if numbers.len() < 4 {
            continue;
        }
        // Blocks are 1024 bytes under `-P`, which is what POSIX fixes them at.
        let block = |s: &str| s.parse::<u64>().ok().map(|n| n * 1024);
        let (Some(size), Some(used), Some(free)) =
            (block(numbers[0]), block(numbers[1]), block(numbers[2]))
        else {
            continue;
        };
        let capacity = numbers[3].trim_end_matches('%').parse::<u8>().unwrap_or(0);
        // Everything left on the line, spaces included.
        let mount = fields.collect::<Vec<_>>().join(" ");
        if mount.is_empty() {
            continue;
        }
        out.push(Filesystem {
            source: source.to_string(),
            size,
            used,
            free,
            capacity,
            mount,
        });
    }
    out
}

/// Turn one filesystem into the table Lua sees.
pub fn filesystem_row(fs: &Filesystem) -> Value {
    let mut t = Table::new();
    let mut set = |key: &str, value: Value| t.set(Value::str(key), value);
    set("source", Value::str(&fs.source));
    set("mount", Value::str(&fs.mount));
    set("size", Value::int(fs.size as i64));
    set("size_human", Value::str(human(fs.size)));
    set("used", Value::int(fs.used as i64));
    set("used_human", Value::str(human(fs.used)));
    set("free", Value::int(fs.free as i64));
    set("free_human", Value::str(human(fs.free)));
    set("capacity", Value::int(fs.capacity as i64));
    Value::Table(Rc::new(RefCell::new(t)))
}

/// The tools that answer in rows, and the function that produces them.
///
/// Returns `None` for every other name, which is what makes `sh.<anything>` still run the external
/// program. Adding a tool is one arm here plus its parser.
/// Whether this command has a row answer at all.
///
/// Asked before the tool runs, so `sh.df` can be handed back as a *function* rather than as the
/// rows themselves — `sh.df()` has to be a call, not an index that already did the work.
pub fn answers_in_rows(command: &str) -> bool {
    matches!(command, "df" | "env" | "ls" | "ps" | "stat")
}

/// A tool's arguments are the words it was called with, exactly as `oslo.run` takes them.
///
/// `sh.stat("a b")` passes one argument holding a space, because there is no shell parse in the
/// middle — the same property that makes `oslo.run{"rm", name}` safe. A tool that ignores its
/// arguments (`df`, `env`) simply does not read them.
pub fn row_answer(
    command: &str,
    env: &std::sync::Arc<std::sync::Mutex<crate::env::Environment>>,
    args: &[String],
) -> Option<Value> {
    match command {
        "df" => {
            // `-P` is the POSIX format: one filesystem per line, fixed column order. Asked for
            // explicitly rather than trusting the default, which wraps a long source onto its own
            // line and shifts every field after it.
            let out = capture(env, &["df", "-P"])?;
            Some(df_rows(&out))
        }
        "env" => Some(env_rows(env)),
        // `ls` answers the current directory. An argument-taking form is the next step and is
        // deliberately not guessed at here.
        "ls" => Some(ls_rows(args.first().map(String::as_str).unwrap_or("."))),
        "ps" => Some(ps_rows()),
        "stat" => Some(stat_rows(args)),
        _ => None,
    }
}

/// `sh.ps()` — the processes on this machine, read from `/proc`.
///
/// Read directly rather than parsed out of `ps` output: the column set `ps` prints differs between
/// implementations and between invocations, so a parser would be guessing at which machine it is
/// on. `/proc` is the same everywhere oslo runs, which is Linux.
pub(crate) fn ps_rows() -> Value {
    let mut list = Table::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Value::Table(Rc::new(RefCell::new(list)));
    };
    let mut found: Vec<(i64, String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(pid) = name.parse::<i64>() else {
            continue;
        };
        // `comm` is the process name; `cmdline` is the full argv with NUL separators. A kernel
        // thread has an empty `cmdline`, which is how it is told apart from a process.
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .map(|s| s.trim_end().to_string())
            .unwrap_or_default();
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"))
            .map(|b| {
                String::from_utf8_lossy(&b)
                    .split('\0')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        if comm.is_empty() {
            continue;
        }
        found.push((pid, comm, cmdline));
    }
    found.sort_by_key(|(pid, _, _)| *pid);
    for (i, (pid, comm, cmdline)) in found.iter().enumerate() {
        let mut row = Table::new();
        row.set(Value::str("pid"), Value::int(*pid));
        row.set(Value::str("name"), Value::str(comm));
        row.set(Value::str("cmdline"), Value::str(cmdline));
        row.set(Value::str("is_kernel"), Value::Bool(cmdline.is_empty()));
        list.set(
            Value::int(i as i64 + 1),
            Value::Table(Rc::new(RefCell::new(row))),
        );
    }
    Value::Table(Rc::new(RefCell::new(list)))
}

/// `sh.stat(path, …)` — one row per path.
///
/// A path that cannot be stat'd contributes a row with `exists = false` rather than being dropped:
/// a caller asking about five paths wants five answers, and a short list would silently misalign
/// with the list it asked about.
fn stat_rows(paths: &[String]) -> Value {
    let mut list = Table::new();
    for (i, path) in paths.iter().enumerate() {
        let mut row = Table::new();
        row.set(Value::str("path"), Value::str(path));
        match std::fs::symlink_metadata(path) {
            Ok(meta) => {
                use std::os::unix::fs::MetadataExt;
                row.set(Value::str("exists"), Value::Bool(true));
                row.set(Value::str("size"), Value::int(meta.len() as i64));
                row.set(Value::str("size_human"), Value::str(human(meta.len())));
                row.set(Value::str("mode"), Value::int(meta.mode() as i64));
                row.set(Value::str("uid"), Value::int(meta.uid() as i64));
                row.set(Value::str("gid"), Value::int(meta.gid() as i64));
                row.set(Value::str("mtime"), Value::int(meta.mtime()));
                row.set(Value::str("is_dir"), Value::Bool(meta.is_dir()));
                row.set(
                    Value::str("is_symlink"),
                    Value::Bool(meta.file_type().is_symlink()),
                );
            }
            Err(_) => {
                row.set(Value::str("exists"), Value::Bool(false));
            }
        }
        list.set(
            Value::int(i as i64 + 1),
            Value::Table(Rc::new(RefCell::new(row))),
        );
    }
    Value::Table(Rc::new(RefCell::new(list)))
}

/// `sh.env()` — the environment as rows, answered from the shell rather than from `env`'s text.
///
/// Parsing `env` output is impossible in general: a value may contain a newline or an `=`, and
/// nothing in the text distinguishes that from the next variable. The shell already holds the
/// pairs, so it hands them over rather than rendering and re-reading them.
fn env_rows(env: &std::sync::Arc<std::sync::Mutex<crate::env::Environment>>) -> Value {
    let mut list = Table::new();
    let Ok(guard) = env.lock() else {
        return Value::Table(Rc::new(RefCell::new(list)));
    };
    let mut pairs: Vec<(String, String)> = guard
        .get_all_vars()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // Sorted, because table iteration has no order and a script that prints them should not
    // produce a different listing on every run.
    pairs.sort();
    for (i, (name, value)) in pairs.iter().enumerate() {
        let mut row = Table::new();
        row.set(Value::str("name"), Value::str(name));
        row.set(Value::str("value"), Value::str(value));
        list.set(
            Value::int(i as i64 + 1),
            Value::Table(Rc::new(RefCell::new(row))),
        );
    }
    Value::Table(Rc::new(RefCell::new(list)))
}

/// `sh.ls()` — a directory as rows.
///
/// The text form of `ls` is genuinely ambiguous for a filename containing a newline, which is a
/// legal filename. Rows have no such problem.
pub(crate) fn ls_rows(dir: &str) -> Value {
    let mut list = Table::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Value::Table(Rc::new(RefCell::new(list)));
    };
    let mut names: Vec<(String, std::fs::Metadata)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            e.metadata().ok().map(|m| (name, m))
        })
        .collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    for (i, (name, meta)) in names.iter().enumerate() {
        let mut row = Table::new();
        row.set(Value::str("name"), Value::str(name));
        row.set(Value::str("size"), Value::int(meta.len() as i64));
        row.set(Value::str("size_human"), Value::str(human(meta.len())));
        row.set(Value::str("is_dir"), Value::Bool(meta.is_dir()));
        row.set(
            Value::str("mode"),
            Value::int(std::os::unix::fs::MetadataExt::mode(meta) as i64),
        );
        list.set(
            Value::int(i as i64 + 1),
            Value::Table(Rc::new(RefCell::new(row))),
        );
    }
    Value::Table(Rc::new(RefCell::new(list)))
}

/// Run a command and answer its standard output, or `None` if it could not run.
fn capture(
    env: &std::sync::Arc<std::sync::Mutex<crate::env::Environment>>,
    argv: &[&str],
) -> Option<String> {
    let mut guard = env.lock().ok()?;
    let words: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let outcome = crate::exec::argv::run(
        &mut guard,
        &words,
        crate::exec::argv::Capture {
            stdout: true,
            stderr: false,
        },
    )
    .ok()?;
    outcome.out
}

/// The list of rows `sh.df()` answers.
pub fn df_rows(output: &str) -> Value {
    let mut list = Table::new();
    for (i, fs) in parse_df(output).iter().enumerate() {
        list.set(Value::int(i as i64 + 1), filesystem_row(fs));
    }
    Value::Table(Rc::new(RefCell::new(list)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Filesystem     1024-blocks      Used Available Capacity Mounted on
/dev/sda1         10485760   5242880   5242880      50% /
tmpfs                65536         0     65536       0% /dev/shm
/dev/sdb1          1048576    524288    524288      50% /mnt/my disk
";

    #[test]
    fn sizes_read_the_way_df_h_reads() {
        assert_eq!(human(0), "0B");
        assert_eq!(human(4300), "4.2K");
        assert_eq!(human(10 * 1024 * 1024 * 1024), "10G");
    }

    /// The reason this parser exists: a mount point may contain spaces, so it is the rest of the
    /// line rather than a field. `df | awk '{print $6}'` gets `/mnt/my` and drops the rest.
    #[test]
    fn a_mount_point_with_spaces_survives() {
        let rows = parse_df(SAMPLE);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].mount, "/mnt/my disk");
        assert_eq!(rows[2].source, "/dev/sdb1");
    }

    /// Blocks are 1024 bytes under `-P`, and the fields carry byte counts rather than renderings.
    #[test]
    fn the_numbers_are_bytes_not_blocks() {
        let rows = parse_df(SAMPLE);
        assert_eq!(rows[0].size, 10 * 1024 * 1024 * 1024);
        assert_eq!(rows[0].free, 5 * 1024 * 1024 * 1024);
        assert_eq!(rows[0].capacity, 50);
        assert_eq!(rows[1].used, 0);
    }

    /// A line that is not a filesystem is skipped rather than producing a row of zeroes — a
    /// wrapped source line, or the blank line some implementations end with.
    #[test]
    fn unparseable_lines_are_skipped() {
        assert!(parse_df("Filesystem 1024-blocks Used Available Capacity Mounted on\n").is_empty());
        assert!(parse_df("").is_empty());
        // A source on its own line, which is what `-P` exists to prevent, contributes nothing
        // rather than half a row.
        assert!(parse_df("header\n/dev/very-long-name-here\n").is_empty());
    }

    #[test]
    fn a_row_carries_both_the_number_and_the_string() {
        let rows = parse_df(SAMPLE);
        let Value::Table(row) = filesystem_row(&rows[0]) else {
            panic!("a row is a table")
        };
        let row = row.borrow();
        let int = |k: &str| row.get(&Value::str(k)).as_number().and_then(|n| n.as_int());
        let text = |k: &str| match row.get(&Value::str(k)) {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        };
        assert_eq!(int("free"), Some(5 * 1024 * 1024 * 1024));
        assert_eq!(text("free_human").as_deref(), Some("5.0G"));
        assert_eq!(text("mount").as_deref(), Some("/"));
    }

    #[test]
    fn the_row_list_is_a_lua_sequence() {
        let Value::Table(list) = df_rows(SAMPLE) else {
            panic!("a list is a table")
        };
        assert_eq!(list.borrow().sequence().len(), 3);
    }
}
