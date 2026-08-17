//! `oslo.fs.watch` — being told when a file changes, instead of asking.
//!
//! ```lua
//! local watch <close> = oslo.fs.watch("src", { "write", "create", "delete" })
//! oslo.every(500, function()
//!   for change in watch do
//!     if change.name:match("%.rs$") then oslo.spawn{ "cargo", "check" } end
//!   end
//! end)
//! ```
//!
//! # Why polling is the interface, and not a callback
//!
//! The kernel can wake a process the instant a file changes, and a callback is what everyone
//! reaches for. oslo cannot honestly offer one: **a Lua handler runs only at a safe point** — a
//! command boundary or an idle prompt, the two moments the shell is holding nothing — which is the
//! same reason `oslo.after` fires between commands rather than on the second. See
//! [`super::timer`]. A callback delivered "when the file changes" would either be a lie about
//! *when*, or would mean re-entering the VM from the middle of an expansion.
//!
//! So this is a queue with a lid on it. The kernel fills it whether or not anyone is looking; the
//! handle drains it when a timer or a hook gets round to it, and nothing is lost in between — which
//! is the part a `stat`-and-compare loop cannot do at all, because it can only see the state a file
//! ended in, never that it was touched twice.
//!
//! # Non-blocking, always
//!
//! The instance is opened `IN_NONBLOCK`, so draining an empty queue answers `nil` immediately
//! rather than parking the shell until somebody happens to save a file. A blocking watch in a shell
//! is a hung shell.
//!
//! # What it does not do
//!
//! **It does not recurse.** inotify watches one directory, not a tree, and adding a watch per
//! subdirectory means tracking creations and deletions of directories to keep the set right — plus
//! a per-watch kernel cost that a `node_modules` would take straight through
//! `max_user_watches`. Naming the directories you care about is the honest version, and
//! `oslo.fs.walk` is how a caller who really wants a tree enumerates one.

use super::util::{failed_path, ok, put, record, text};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};
use oslo_base::value::{LuaError, Table, Value};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

/// The names a caller writes, and the flags they mean.
///
/// **Shorter than inotify's own, and fewer.** `IN_CLOSE_WRITE` is what "the file was saved" almost
/// always means — `IN_MODIFY` fires per `write(2)`, so a large save arrives as a burst — and a
/// name-per-constant surface would make the caller learn inotify to ask a simple question.
const KINDS: &[(&str, AddWatchFlags)] = &[
    // Saved: opened for writing, then closed. One event per save, which is what people mean.
    ("write", AddWatchFlags::IN_CLOSE_WRITE),
    // Every `write(2)`, for a caller that genuinely wants the partial ones.
    ("modify", AddWatchFlags::IN_MODIFY),
    ("create", AddWatchFlags::IN_CREATE),
    ("delete", AddWatchFlags::IN_DELETE),
    ("move", AddWatchFlags::IN_MOVE),
    ("attrib", AddWatchFlags::IN_ATTRIB),
    ("open", AddWatchFlags::IN_OPEN),
    ("read", AddWatchFlags::IN_ACCESS),
];

/// What a watch is holding: the instance, which paths its descriptors name, and what has arrived.
struct Watching {
    inotify: Inotify,
    /// Watch descriptor to the directory it was added for, so an event can say where it happened.
    where_: HashMap<WatchDescriptor, String>,
    /// Events read from the kernel but not yet handed to Lua.
    ///
    /// **A queue, because one read returns many.** `read_events` drains whatever the kernel has
    /// buffered, and the iterator hands over one at a time; without somewhere to put the rest, a
    /// burst of twenty saves would be nineteen events thrown away.
    pending: RefCell<VecDeque<Value>>,
}

/// Add `oslo.fs.watch` to the `oslo.fs` table.
pub fn install(fs: &mut Table) {
    // oslo.fs.watch(path, { "write", … }) -> a handle, or nil + message
    put(fs, "watch", |_, args| {
        let path = text(&args, 1, "oslo.fs.watch")?;
        let flags = wanted(args.get(1))?;
        let inotify = match Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC) {
            Ok(it) => it,
            Err(e) => return failed_path(&path, &std::io::Error::from(e)),
        };
        let descriptor = match inotify.add_watch(path.as_str(), flags) {
            Ok(wd) => wd,
            Err(e) => return failed_path(&path, &std::io::Error::from(e)),
        };
        let mut where_ = HashMap::new();
        where_.insert(descriptor, path.clone());
        ok(handle(Rc::new(Watching {
            inotify,
            where_,
            pending: RefCell::new(VecDeque::new()),
        })))
    });
}

/// The handle `watch` answers with: an iterator over what has happened, and a thing that closes.
fn handle(watching: Rc<Watching>) -> Value {
    let mut handle = super::handle::Handle::new("oslo.fs.watch");

    // Callable, so `for change in watch do` works — see `super::handle::Handle::calls`. `nil` means
    // "nothing right now", not "never again", which is what makes it drainable from a timer.
    let it = Rc::clone(&watching);
    handle.calls("oslo.fs.watch", move |_, _| ok(it.next_change()));

    // oslo.fs.watch(…):path() -> the directory being watched
    let it = Rc::clone(&watching);
    handle.verb("path", move |_, _| {
        ok(match it.where_.values().next() {
            Some(path) => Value::str(path),
            None => Value::Nil,
        })
    });

    // Closing drops the inotify instance, which is what releases the kernel's watch. Without it a
    // watch outlives every reference to it for the rest of the session.
    handle.on_close("oslo.fs.watch.close", move || {
        watching.pending.borrow_mut().clear();
    });

    handle.build()
}

impl Watching {
    /// The next change, reading more from the kernel only when nothing is queued.
    fn next_change(&self) -> Value {
        if let Some(ready) = self.pending.borrow_mut().pop_front() {
            return ready;
        }
        // `WouldBlock` is the ordinary answer for "nothing has happened", not a failure — the
        // instance is non-blocking on purpose.
        let Ok(events) = self.inotify.read_events() else {
            return Value::Nil;
        };
        {
            let mut pending = self.pending.borrow_mut();
            for event in events {
                let kind = names_of(event.mask);
                // `IN_IGNORED` is the kernel saying the watch is gone — the directory was removed
                // or unmounted. Passed on rather than swallowed, because a caller looping on this
                // otherwise waits forever for events that can no longer come.
                pending.push_back(record(vec![
                    (
                        "name",
                        match &event.name {
                            Some(name) => Value::str(name.to_string_lossy()),
                            None => Value::Nil,
                        },
                    ),
                    (
                        "path",
                        match self.where_.get(&event.wd) {
                            Some(dir) => Value::str(dir),
                            None => Value::Nil,
                        },
                    ),
                    ("kind", Value::str(kind)),
                    (
                        "directory",
                        Value::Bool(event.mask.contains(AddWatchFlags::IN_ISDIR)),
                    ),
                ]));
            }
        }
        self.pending.borrow_mut().pop_front().unwrap_or(Value::Nil)
    }
}

/// The flags a `{ "write", "create" }` list asks for.
///
/// No list means every kind this module names — the useful default for "tell me when anything
/// happens here", and the one a caller writes first while finding out what they want.
fn wanted(value: Option<&Value>) -> Result<AddWatchFlags, LuaError> {
    let Some(Value::Table(asked)) = value else {
        return Ok(KINDS
            .iter()
            .fold(AddWatchFlags::empty(), |all, (_, flag)| all | *flag));
    };
    let mut flags = AddWatchFlags::empty();
    for entry in asked.borrow().sequence() {
        let Value::Str(name) = entry else {
            return Err(LuaError::new(
                "oslo.fs.watch: the kinds are a list of strings".to_string(),
            ));
        };
        match KINDS.iter().find(|(known, _)| *known == name.as_ref()) {
            Some((_, flag)) => flags |= *flag,
            None => {
                let known: Vec<&str> = KINDS.iter().map(|(name, _)| *name).collect();
                return Err(LuaError::new(format!(
                    "oslo.fs.watch: {name:?} is not a kind; they are {}",
                    known.join(", ")
                )));
            }
        }
    }
    Ok(flags)
}

/// The name for what happened, as the first kind whose flag is set.
///
/// One name rather than a list: a caller branches on what a change *was*, and inotify sets at most
/// one of these per event anyway. `IN_ISDIR` is carried separately, as a field, because it modifies
/// every one of them rather than being one of them.
fn names_of(mask: AddWatchFlags) -> &'static str {
    for (name, flag) in KINDS {
        if mask.contains(*flag) {
            return name;
        }
    }
    if mask.contains(AddWatchFlags::IN_IGNORED) || mask.contains(AddWatchFlags::IN_UNMOUNT) {
        return "gone";
    }
    "other"
}

#[cfg(test)]
#[path = "watch/tests.rs"]
mod tests;
