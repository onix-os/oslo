//! `oslo.fs` — the filesystem, answering with tables rather than text.
//!
//! `oslo.fs.ls(dir)` gives entries with `name`, `size`, `type` and `mtime`. That is the whole
//! difference from shelling out to `ls -l` and parsing it: no column alignment to get wrong, no
//! locale to change the date format underneath you, no filename with a space in it to split at.
//!
//! Every call here can fail for a reason outside the caller's control — the file is gone, the
//! directory is not readable — so every call answers `nil, message` rather than raising. See
//! [`super::util::failed`].

use super::util::{
    failed, failed_between, failed_path, int, list, ok, opt_text, put, raw, record, text,
};
use oslo_base::value::{LuaError, Table, Value};
use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::rc::Rc;
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
            // **The bytes, exactly.** This went through `String::from_utf8_lossy` until Lua
            // strings could be bytes at oslo`s end too, so reading a PNG answered a PNG with every
            // non-text byte replaced by `U+FFFD` — a read that looked like it had worked. Text is
            // still text; only what was never text is now itself. See `oslo_base::value::Value`.
            Ok(bytes) => ok(Value::bytes(&bytes)),
            Err(e) => failed_path(&path, &e),
        }
    });

    // oslo.fs.lines(path) -> an iterator over the file's lines
    //
    // **The descriptor stays open between calls**, which is what makes this the right way to read
    // a log. It used to read the whole file and answer a table, because there was no `__close` to
    // shut a held descriptor with; there is one now.
    put(it, "lines", |_, args| {
        let path = text(&args, 1, "oslo.fs.lines")?;
        match fs::File::open(&path) {
            Ok(file) => ok(reader(std::io::BufReader::new(file))),
            Err(e) => failed_path(&path, &e),
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
        let contents = raw(&args, 2, "oslo.fs.write")?;
        match fs::write(&path, contents) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed_path(&path, &e),
        }
    });

    put(it, "append", |_, args| {
        use std::io::Write;
        let path = text(&args, 1, "oslo.fs.append")?;
        let contents = raw(&args, 2, "oslo.fs.append")?;
        let opened = fs::OpenOptions::new().create(true).append(true).open(&path);
        match opened.and_then(|mut f| f.write_all(&contents)) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed_path(&path, &e),
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
            Err(e) => failed_path(&path, &e),
        }
    });

    // oslo.fs.remove(path, recursive)
    put(it, "remove", |_, args| {
        let path = text(&args, 1, "oslo.fs.remove")?;
        let recursive = args.get(1).is_some_and(Value::truthy);
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) => return failed_path(&path, &e),
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
            Err(e) => failed_path(&path, &e),
        }
    });

    put(it, "rename", |_, args| {
        let from = text(&args, 1, "oslo.fs.rename")?;
        let to = text(&args, 2, "oslo.fs.rename")?;
        match fs::rename(&from, &to) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed_between(&from, &to, &e),
        }
    });

    put(it, "copy", |_, args| {
        let from = text(&args, 1, "oslo.fs.copy")?;
        let to = text(&args, 2, "oslo.fs.copy")?;
        match fs::copy(&from, &to) {
            Ok(bytes) => ok(Value::int(bytes as i64)),
            Err(e) => failed_between(&from, &to, &e),
        }
    });

    put(it, "symlink", |_, args| {
        let target = text(&args, 1, "oslo.fs.symlink")?;
        let link = text(&args, 2, "oslo.fs.symlink")?;
        match std::os::unix::fs::symlink(&target, &link) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed_path(&link, &e),
        }
    });

    // oslo.fs.chmod(path, 0755) — the mode is a number, as `chmod` and `stat` both speak it.
    put(it, "chmod", |_, args| {
        let path = text(&args, 1, "oslo.fs.chmod")?;
        let mode = int(&args, 2, "oslo.fs.chmod")?;
        match fs::set_permissions(&path, fs::Permissions::from_mode(mode as u32)) {
            Ok(()) => ok(Value::Bool(true)),
            Err(e) => failed_path(&path, &e),
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

    // oslo.fs.mktempdir(prefix) -> a handle on a directory that did not exist a moment ago
    //
    // **A handle rather than a path, because a temporary directory is the one thing in `oslo.fs`
    // with a lifetime.** Every other call here acts on a path somebody else owns; this one *makes*
    // something, and until there was `<close>` there was nowhere to say when it should go:
    //
    //     local tmp <close> = oslo.fs.mktempdir()
    //     oslo.fs.write(tmp.path .. "/notes", body)
    //
    // `tostring(tmp)` is the path, so a handle reads as one wherever a message wants it.
    put(it, "mktempdir", |_, args| {
        let prefix = opt_text(&args, 1, "oslo.fs.mktempdir")?.unwrap_or_else(|| "oslo".to_string());
        match unique_temp(&prefix, true) {
            Ok(path) => ok(tempdir(path)),
            Err(e) => failed("mktempdir", e),
        }
    });
}

/// The handle `mktempdir` answers with.
fn tempdir(path: String) -> Value {
    let mut handle = super::handle::Handle::new("oslo.fs.tempdir");
    handle.field("path", Value::str(&path)).shows(&path);

    let it = path.clone();
    handle.verb("remove", move |_, _| {
        ok(Value::Bool(fs::remove_dir_all(&it).is_ok()))
    });

    // **`<close>` removes it, and the collector does not.** `remove_dir_all` on a path a config
    // still means to use is not a mistake anything could recover from, and a handle whose `.path`
    // has been copied elsewhere looks unreachable to the collector while the directory is still in
    // use. Leaving it for the system to clean is the safe half of that trade.
    handle.on_close("oslo.fs.tempdir.close", move || {
        let _ = fs::remove_dir_all(&path);
    });

    handle.build()
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
            Err(e) => return failed_path(&dir, &e),
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

    // oslo.fs.walk(dir) -> an iterator over every path under `dir`, depth first, directories
    // before their contents
    //
    // **Lazy, because a tree has no size you can promise.** The table this used to answer was the
    // whole of `/nix/store` before the first line of the loop ran; the iterator opens one directory
    // at a time and stops the moment the loop does.
    //
    // Symlinks are not followed. A link back up the tree is what turns "walk this directory" into
    // an infinite loop, and a script written against a tree it does not control will meet one.
    put(it, "walk", |_, args| {
        let root = opt_text(&args, 1, "oslo.fs.walk")?.unwrap_or_else(|| ".".to_string());
        match fs::read_dir(&root) {
            Ok(reading) => ok(walker(reading)),
            Err(e) => failed_path(&root, &e),
        }
    });

    // oslo.fs.glob(pattern) — the shell's own globber, so the two languages agree about what
    // `*.conf` means down to the last edge case.
    put(it, "glob", |_, args| {
        let pattern = text(&args, 1, "oslo.fs.glob")?;
        let field = [oslo_shell::expand::Run::new(
            pattern.clone(),
            oslo_shell::expand::Origin::Literal,
        )];
        let matches = oslo_shell::expand::glob::expand_glob(&field);
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
            Err(e) => failed_path(&path, &e),
        }
    });

    put(it, "realpath", |_, args| {
        let path = text(&args, 1, "oslo.fs.realpath")?;
        match fs::canonicalize(&path) {
            Ok(resolved) => ok(Value::str(resolved.to_string_lossy())),
            Err(e) => failed_path(&path, &e),
        }
    });

    put(it, "readlink", |_, args| {
        let path = text(&args, 1, "oslo.fs.readlink")?;
        match fs::read_link(&path) {
            Ok(target) => ok(Value::str(target.to_string_lossy())),
            Err(e) => failed_path(&path, &e),
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
/// The iterator `oslo.fs.lines` answers: one line per call, `nil` at the end.
///
/// A trailing newline ends the last line rather than starting an empty one, which is what `wc -l`
/// counts and what a caller means by "the lines of this file".
fn reader(file: std::io::BufReader<fs::File>) -> Value {
    let source = Rc::new(RefCell::new(Some(file)));
    let mut handle = super::handle::Handle::new("oslo.fs.lines");

    let it = Rc::clone(&source);
    handle.calls("oslo.fs.lines", move |_, _| {
        use std::io::BufRead;
        let mut slot = it.borrow_mut();
        let Some(buffered) = slot.as_mut() else {
            return ok(Value::Nil);
        };
        let mut line = Vec::new();
        match buffered.read_until(b'\n', &mut line) {
            Ok(0) => {
                *slot = None;
                ok(Value::Nil)
            }
            Ok(_) => {
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                ok(Value::bytes(&line))
            }
            Err(e) => {
                *slot = None;
                Err(LuaError::new(format!("oslo.fs.lines: {e}")))
            }
        }
    });

    handle.on_close("oslo.fs.lines.close", move || {
        source.borrow_mut().take();
    });

    handle.build()
}

/// The iterator `oslo.fs.walk` answers: a handle that is also callable.
///
/// ```lua
/// for path in oslo.fs.walk("/etc") do print(path) end
///
/// local tree <close> = oslo.fs.walk("/nix/store")   -- the open directories are let go here
/// for path in tree do if path:find("cache") then break end end
/// ```
///
/// **A stack of open directories rather than recursion**, because the recursion was what made this
/// eager: a function that has to return before the caller sees anything cannot answer one path at a
/// time. One `ReadDir` per level is also one file descriptor per level, which is why the handle
/// closes — a loop abandoned deep in a tree is holding them until it does. See
/// [`super::handle::Handle::calls`].
fn walker(root: fs::ReadDir) -> Value {
    let stack = Rc::new(RefCell::new(vec![root]));
    let mut handle = super::handle::Handle::new("oslo.fs.walk");

    let it = Rc::clone(&stack);
    handle.calls("oslo.fs.walk", move |_, _| {
        let mut stack = it.borrow_mut();
        loop {
            let Some(level) = stack.last_mut() else {
                return ok(Value::Nil);
            };
            let Some(entry) = level.next() else {
                stack.pop();
                continue;
            };
            let entry = entry.map_err(|e| LuaError::new(format!("oslo.fs.walk: {e}")))?;
            let path = entry.path();
            // `symlink_metadata`, so a symlink to a parent directory is listed and not descended
            // into. Following it is how a walk never finishes.
            let is_dir = path
                .symlink_metadata()
                .map_err(|e| LuaError::new(format!("oslo.fs.walk: {}: {e}", path.display())))?
                .is_dir();
            if is_dir {
                // Pushed before the directory itself is answered, so the next call descends —
                // which is what makes it depth first, with a directory before its contents.
                let below = fs::read_dir(&path)
                    .map_err(|e| LuaError::new(format!("oslo.fs.walk: {}: {e}", path.display())))?;
                stack.push(below);
            }
            return ok(Value::str(path.to_string_lossy()));
        }
    });

    handle.on_close("oslo.fs.walk.close", move || stack.borrow_mut().clear());

    handle.build()
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
