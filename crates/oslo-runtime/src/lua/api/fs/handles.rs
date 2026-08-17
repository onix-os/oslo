//! The three things `oslo.fs` hands out that *own* something: an open file, a walk part-way down a
//! tree, and a temporary directory.
//!
//! Split from `fs.rs` because they are a subject of their own — the rest of that module is calls
//! that act on a path and are finished when they return, and these are the ones with a lifetime.
//! Each is a [`super::super::handle::Handle`]: callable through `__call` where a generic `for` wants an
//! iterator, and closing through `__close` where the resource has to be let go of.

use super::super::util::ok;
use oslo_base::value::{LuaError, Value};
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

/// The handle `mktempdir` answers with.
pub(super) fn tempdir(path: String) -> Value {
    let mut handle = super::super::handle::Handle::new("oslo.fs.tempdir");
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

/// The iterator `oslo.fs.lines` answers: one line per call, `nil` at the end.
///
/// A trailing newline ends the last line rather than starting an empty one, which is what `wc -l`
/// counts and what a caller means by "the lines of this file".
pub(super) fn reader(file: std::io::BufReader<fs::File>) -> Value {
    let source = Rc::new(RefCell::new(Some(file)));
    let mut handle = super::super::handle::Handle::new("oslo.fs.lines");

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
/// [`super::super::handle::Handle::calls`].
pub(super) fn walker(root: fs::ReadDir) -> Value {
    let stack = Rc::new(RefCell::new(vec![root]));
    let mut handle = super::super::handle::Handle::new("oslo.fs.walk");

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
