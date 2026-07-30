//! `oslo.fs` — the filesystem, answering with tables rather than text.
//!
//! `oslo.fs.ls(dir)` gives entries with `name`, `size`, `type` and `mtime`. That is the whole
//! difference from shelling out to `ls -l` and parsing it: no column alignment to get wrong, no
//! locale to change the date format underneath you, no filename with a space in it to split at.
//!
//! Every call here can fail for a reason outside the caller's control — the file is gone, the
//! directory is not readable — so every call answers `nil, message` rather than raising. See
//! [`super::util::failed`].

use super::util::{failed, int, list, ok, opt_text, put, record, text};
use crate::lua::eval::value::{Table, Value};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::time::UNIX_EPOCH;

pub fn build() -> Value {
    let mut it = Table::new();

    reading(&mut it);
    writing(&mut it);
    listing(&mut it);
    metadata(&mut it);

    Value::table(it)
}

fn reading(it: &mut Table) {
    // oslo.fs.read(path) -> contents, or nil + message
    put(it, "read", |_, args| {
        let path = text(&args, 1, "oslo.fs.read")?;
        match fs::read(&path) {
            // Lossy rather than refusing: a shell reads config files and log files, and one stray
            // byte in a mostly-text file should not make the whole thing unreadable. A caller who
            // needs the bytes exactly is reading something this API is the wrong tool for.
            Ok(bytes) => ok(Value::str(String::from_utf8_lossy(&bytes))),
            Err(e) => failed(&path, e),
        }
    });

    // oslo.fs.lines(path) -> {"first", "second", ...}
    //
    // Reads the whole file. Streaming would need an iterator holding an open descriptor across
    // Lua calls, and this evaluator has no `__close` to shut one; the honest version comes with
    // `oslo.lines`, which is a live command's output rather than a file.
    put(it, "lines", |_, args| {
        let path = text(&args, 1, "oslo.fs.lines")?;
        match fs::read(&path) {
            Ok(bytes) => {
                let content = String::from_utf8_lossy(&bytes);
                // A trailing newline ends the last line rather than starting an empty one, which
                // is what `wc -l` counts and what a caller means by "the lines of this file".
                let body = content.strip_suffix('\n').unwrap_or(&content);
                if body.is_empty() {
                    return ok(list([]));
                }
                ok(list(body.split('\n').map(Value::str)))
            }
            Err(e) => failed(&path, e),
        }
    });

    put(it, "exists", |_, args| {
        let path = text(&args, 1, "oslo.fs.exists")?;
        // `symlink_metadata`, so a dangling symlink is reported as existing — it does, and
        // `rm` can remove it.
        ok(Value::Bool(fs::symlink_metadata(&path).is_ok()))
    });
}

fn writing(it: &mut Table) {
    // oslo.fs.write(path, contents) -> true, or nil + message
    put(it, "write", |_, args| {
        let path = text(&args, 1, "oslo.fs.write")?;
        let contents = text(&args, 2, "oslo.fs.write")?;
        match fs::write(&path, contents) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed(&path, e),
        }
    });

    put(it, "append", |_, args| {
        use std::io::Write;
        let path = text(&args, 1, "oslo.fs.append")?;
        let contents = text(&args, 2, "oslo.fs.append")?;
        let opened = fs::OpenOptions::new().create(true).append(true).open(&path);
        match opened.and_then(|mut f| f.write_all(contents.as_bytes())) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed(&path, e),
        }
    });

    // oslo.fs.mkdir(path) — always `-p`.
    //
    // One function rather than two, because the plain form is almost never what anyone wants: a
    // script that creates a directory wants it to exist afterwards, not to fail because a parent
    // was missing or because it already succeeded once.
    put(it, "mkdir", |_, args| {
        let path = text(&args, 1, "oslo.fs.mkdir")?;
        match fs::create_dir_all(&path) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed(&path, e),
        }
    });

    // oslo.fs.remove(path, recursive)
    put(it, "remove", |_, args| {
        let path = text(&args, 1, "oslo.fs.remove")?;
        let recursive = args.get(1).is_some_and(crate::lua::eval::Value::truthy);
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) => return failed(&path, e),
        };
        // A symlink is removed as a link, never followed — deleting what a link points at when
        // asked to delete the link is how a cleanup script destroys someone's home directory.
        let removed = if meta.is_dir() && recursive {
            fs::remove_dir_all(&path)
        } else if meta.is_dir() {
            fs::remove_dir(&path)
        } else {
            fs::remove_file(&path)
        };
        match removed {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed(&path, e),
        }
    });

    put(it, "rename", |_, args| {
        let from = text(&args, 1, "oslo.fs.rename")?;
        let to = text(&args, 2, "oslo.fs.rename")?;
        match fs::rename(&from, &to) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed(&format!("{from} -> {to}"), e),
        }
    });

    put(it, "copy", |_, args| {
        let from = text(&args, 1, "oslo.fs.copy")?;
        let to = text(&args, 2, "oslo.fs.copy")?;
        match fs::copy(&from, &to) {
            Ok(bytes) => ok(Value::int(bytes as i64)),
            Err(e) => failed(&format!("{from} -> {to}"), e),
        }
    });

    put(it, "symlink", |_, args| {
        let target = text(&args, 1, "oslo.fs.symlink")?;
        let link = text(&args, 2, "oslo.fs.symlink")?;
        match std::os::unix::fs::symlink(&target, &link) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed(&link, e),
        }
    });

    // oslo.fs.chmod(path, 0755) — the mode is a number, as `chmod` and `stat` both speak it.
    put(it, "chmod", |_, args| {
        let path = text(&args, 1, "oslo.fs.chmod")?;
        let mode = int(&args, 2, "oslo.fs.chmod")?;
        match fs::set_permissions(&path, fs::Permissions::from_mode(mode as u32)) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed(&path, e),
        }
    });

    // oslo.fs.mktemp(prefix) -> a path that did not exist a moment ago
    //
    // The file is created, not just named. Returning a name for the caller to open is the classic
    // temp-file race: between the answer and the open, anything can take the name.
    put(it, "mktemp", |_, args| {
        let prefix = opt_text(&args, 1, "oslo.fs.mktemp")?.unwrap_or_else(|| "oslo".to_string());
        match unique_temp(&prefix, false) {
            Ok(path) => ok(Value::str(path)),
            Err(e) => failed("mktemp", e),
        }
    });

    put(it, "mktempdir", |_, args| {
        let prefix = opt_text(&args, 1, "oslo.fs.mktempdir")?.unwrap_or_else(|| "oslo".to_string());
        match unique_temp(&prefix, true) {
            Ok(path) => ok(Value::str(path)),
            Err(e) => failed("mktempdir", e),
        }
    });
}

fn listing(it: &mut Table) {
    // oslo.fs.ls(dir) -> { {name=…, size=…, type=…, mtime=…}, … }
    //
    // Sorted by name, because a directory's order on disk is arbitrary and a script that prints
    // one unsorted looks broken every time the filesystem reorders it.
    put(it, "ls", |_, args| {
        let dir = opt_text(&args, 1, "oslo.fs.ls")?.unwrap_or_else(|| ".".to_string());
        let read = match fs::read_dir(&dir) {
            Ok(read) => read,
            Err(e) => return failed(&dir, e),
        };
        let mut entries: Vec<(String, Value)> = Vec::new();
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let described = match entry.path().symlink_metadata() {
                Ok(meta) => describe(&name, &meta),
                // An entry whose metadata cannot be read is still an entry — reporting the name
                // with unknown details beats dropping it from the listing.
                Err(_) => record(vec![("name", Value::str(&name))]),
            };
            entries.push((name, described));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        ok(list(entries.into_iter().map(|(_, v)| v)))
    });

    // oslo.fs.walk(dir) -> every path under `dir`, depth first, directories before their contents
    //
    // Symlinks are not followed. A link back up the tree is what turns "walk this directory" into
    // an infinite loop, and a script written against a tree it does not control will meet one.
    put(it, "walk", |_, args| {
        let root = opt_text(&args, 1, "oslo.fs.walk")?.unwrap_or_else(|| ".".to_string());
        let mut found = Vec::new();
        if let Err(e) = walk(Path::new(&root), &mut found) {
            return failed(&root, e);
        }
        ok(list(found.into_iter().map(Value::str)))
    });

    // oslo.fs.glob(pattern) — the shell's own globber, so the two languages agree about what
    // `*.conf` means down to the last edge case.
    put(it, "glob", |_, args| {
        let pattern = text(&args, 1, "oslo.fs.glob")?;
        let field = [crate::expand::Run::new(
            pattern.clone(),
            crate::expand::Origin::Literal,
        )];
        let matches = crate::expand::glob::expand_glob(&field);
        // `expand_glob` yields the pattern back when nothing matched, the way an unquoted word
        // does on a command line. Here that would be a lie, so it becomes an empty table.
        let matches = if matches == vec![pattern] {
            Vec::new()
        } else {
            matches
        };
        ok(list(matches.into_iter().map(Value::str)))
    });
}

fn metadata(it: &mut Table) {
    // oslo.fs.stat(path) -> {name, size, type, mtime, mode, uid, gid}
    put(it, "stat", |_, args| {
        let path = text(&args, 1, "oslo.fs.stat")?;
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                let name = Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                ok(describe(&name, &meta))
            }
            Err(e) => failed(&path, e),
        }
    });

    put(it, "realpath", |_, args| {
        let path = text(&args, 1, "oslo.fs.realpath")?;
        match fs::canonicalize(&path) {
            Ok(resolved) => ok(Value::str(resolved.to_string_lossy())),
            Err(e) => failed(&path, e),
        }
    });

    put(it, "readlink", |_, args| {
        let path = text(&args, 1, "oslo.fs.readlink")?;
        match fs::read_link(&path) {
            Ok(target) => ok(Value::str(target.to_string_lossy())),
            Err(e) => failed(&path, e),
        }
    });

    put(it, "cwd", |_, _| match std::env::current_dir() {
        Ok(dir) => ok(Value::str(dir.to_string_lossy())),
        Err(e) => failed("cwd", e),
    });
}

/// One entry, as the table every listing hands back.
fn describe(name: &str, meta: &fs::Metadata) -> Value {
    let kind = if meta.is_dir() {
        "directory"
    } else if meta.is_symlink() {
        "symlink"
    } else if meta.is_file() {
        "file"
    } else {
        // A socket, fifo or device node. Named rather than mislabelled as a file, because a
        // script that opens one expecting a file will hang rather than fail.
        "other"
    };
    record(vec![
        ("name", Value::str(name)),
        ("size", Value::int(meta.len() as i64)),
        ("type", Value::str(kind)),
        ("mtime", Value::int(seconds(meta))),
        // Permission bits only: the type bits are already in `type`, and `0o100644` printed as a
        // mode is what makes people think `chmod` needs six digits.
        (
            "mode",
            Value::int((meta.permissions().mode() & 0o7777) as i64),
        ),
        ("uid", Value::int(meta.uid() as i64)),
        ("gid", Value::int(meta.gid() as i64)),
    ])
}

/// Modification time as a Unix timestamp, which is what a script can compare and format.
fn seconds(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Collect every path under `root`, depth first.
fn walk(root: &Path, found: &mut Vec<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        found.push(path.to_string_lossy().into_owned());
        // `symlink_metadata`, so a symlink to a parent directory is listed and not descended
        // into. Following it is how a walk never finishes.
        if entry.path().symlink_metadata()?.is_dir() {
            walk(&path, found)?;
        }
    }
    Ok(())
}

/// Create a file or directory under `$TMPDIR` whose name nothing else holds.
///
/// The name mixes the process id with a counter, and creation uses `create_new`, so the loop
/// retries rather than trusting the name to be free — two shells started in the same second have
/// different pids, and one shell's two calls have different counters.
fn unique_temp(prefix: &str, directory: bool) -> std::io::Result<String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let pid = std::process::id();
    for _ in 0..1000 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = Path::new(&base).join(format!("{prefix}.{pid}.{n}"));
        let created = if directory {
            fs::create_dir(&path)
        } else {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map(|_| ())
        };
        match created {
            Ok(()) => return Ok(path.to_string_lossy().into_owned()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other(
        "could not find an unused name in 1000 tries",
    ))
}
