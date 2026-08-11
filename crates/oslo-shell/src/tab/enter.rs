//! The key, the finder it opens, and what is done with the answer.
//!
//! This is the whole of the feature that anybody sees. Everything else in this module is the
//! machinery it stands on.
//!
//! # One key, two places, one meaning
//!
//! Outside a tab the shell owns the terminal, so the key is an ordinary editor binding and this is
//! called from a prompt. Inside a tab the client owns it and the shell is on the far side of a pty,
//! so the client takes the key out of the stream and calls the same thing. The finder does not know
//! which happened; only what Esc means differs, and that is the caller's to say.
//!
//! ```text
//!            ┌── pick an existing one ──► attach to it
//!   ^\ ──►  finder ── type a name ──────► make it, attach
//!            └── Esc ──────────────────► outside: nothing. inside: leave the tab running.
//! ```
//!
//! # Why the shell inside is `exec`d rather than forked on
//!
//! `spawn` hands the child a choice: carry on as the shell it already is, or become a new one. Here
//! it must become a new one. The key is pressed at a prompt, long after oslo has started the
//! threads that warm the `$PATH` index — and `fork` carries only the calling thread, so a child
//! that carried on would hold locks belonging to threads that do not exist in it. `exec` costs one
//! more config read, once, when a tab is made; that is the right price for not shipping a deadlock
//! that appears under load.

use super::{backend, client, detach, dir, keeper, name as naming};
use oslo_ui::ask::{Answer, Choice, Pick, pick_or_create};
use std::io;

/// What the key did, so a caller can tell "nothing happened" from "you are somewhere else now".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Went {
    /// The finder was dismissed. Nothing was made, entered or left.
    Nowhere,
    /// A tab was entered and has since been left, or ended.
    ThereAndBack,
}

/// Open the finder and act on the answer, until nothing is left to attach to.
///
/// **Returns only when the terminal is the caller's again**, so a prompt can be redrawn and a
/// client can exit. Everything in between — attaching, the key being pressed again inside, moving
/// to another tab — happens in the loop.
pub fn open(key: &str, replay: u64) -> io::Result<Went> {
    // **Before anything reads or writes there.** Listing touches the directory and so does asking
    // whether a name is alive — which creates its lock file. A check that ran later would have
    // already left a file in a directory it was about to refuse.
    dir::open_checked()?;
    // Read once, here, so a session cannot end up half in one backend and half in the other
    // because the setting changed while a tab was attached.
    let tabs = backend::current(oslo_ui::settings::current().tab.daemon);
    let detach = detach::Key::named(key);
    let mut went = Went::Nowhere;
    let mut next = ask(&*tabs, None)?;

    while let Some(name) = next {
        tabs.ensure(&name)?;
        went = Went::ThereAndBack;
        match client::attach(tabs.connect(&name)?, &name, detach, replay)? {
            // The shell inside exited, so there is nothing to go back to and nothing to ask about.
            client::Left::Ended => return Ok(went),
            // The key, pressed inside. The finder opens on the terminal the client has just handed
            // back, and Esc here means leave — which is the one thing it cannot mean outside.
            client::Left::Detached => next = ask(&*tabs, Some(&name))?,
        }
    }
    Ok(went)
}

/// The finder, listing every tab and offering what is typed as a new one.
///
/// `inside` is the tab the question is being asked from, which **is listed like any other**: going
/// back into the one you are in has to be possible, or the key is a trap for anybody who pressed it
/// by accident.
fn ask(tabs: &dyn backend::Tabs, inside: Option<&str>) -> io::Result<Option<String>> {
    let running = tabs.list()?;
    let spec = Choice {
        items: running.clone(),
        look: look(inside),
        // Short on purpose. This is a list of sessions, not of history — there are as many rows as
        // you have tabs, and a panel sized for a thousand lines would be mostly empty air.
        height: 8,
        ..Choice::default()
    };

    Ok(match pick_or_create(&spec, "new tab {}") {
        Answer::Given(Pick::Chosen(name)) => Some(name),
        // Refused rather than quietly rewritten: a name is part of a path, and a name you did not
        // type is one you cannot find again. See `name::valid`.
        Answer::Given(Pick::New(typed)) if !naming::valid(&typed) => {
            return Err(io::Error::other(format!(
                "{typed:?} is not a usable tab name"
            )));
        }
        Answer::Given(Pick::New(typed)) => Some(typed),
        // Esc. Outside, the caller has nothing to do; inside, this is how you leave.
        Answer::Cancelled => None,
        // No terminal is not a refusal to answer, it is a place the question cannot be asked.
        Answer::NoTerminal => None,
    })
}

/// The history finder's look, because this is the same kind of question.
///
/// **The same renderer, so the two cannot drift.** `Preset::History` is where the striping, the
/// tinted filter row, the match marks and the counts live; a second list with its own idea of those
/// would be a second thing to keep in step with the theme. Only what the preset cannot know is set
/// here — which tab you are asking from, and that the list is short.
fn look(inside: Option<&str>) -> oslo_ui::ask::look::Look {
    let mut look = oslo_ui::ask::Preset::History.look();
    look.badge = inside.unwrap_or("tab").to_string();
    // `[work] || 2/3` — where you are and how much of the list you are seeing, on the right, since
    // both are facts about what you are looking at rather than part of what you are typing.
    look.right = "{badge} || {n}/{total} ".to_string();
    look.placeholder = "type to filter, or a name for a new one".to_string();
    look
}

/// Carry on as the caller, or — in the process that turned out to be the shell — become one.
pub(super) fn become_shell_or(role: keeper::Role, name: &str) -> io::Result<()> {
    match role {
        keeper::Role::Caller(_) => Ok(()),
        keeper::Role::Inside => exec_inside(name),
    }
}

/// Become the shell inside a tab. Never returns.
///
/// The environment is marked here rather than by the caller, so every path into a tab agrees on
/// what `$TAB` says.
pub(super) fn exec_inside(name: &str) -> ! {
    // SAFETY: a fresh fork about to `exec` — single-threaded, and nothing else can be reading the
    // environment while it is written.
    unsafe { std::env::set_var(INSIDE, name) };
    // `/proc/self/exe` rather than `$SHELL` or a name on `$PATH`: a tab is an oslo, and the one
    // that made it is the one to run.
    let me = std::ffi::CString::new("/proc/self/exe").unwrap_or_default();
    let argv = [
        std::ffi::CString::new("oslo").unwrap_or_default(),
        std::ffi::CString::new("-i").unwrap_or_default(),
    ];
    let _ = nix::unistd::execv(&me, &argv);
    // Only reached if the exec failed, and this process must never return into the caller and
    // start behaving like the shell that made it.
    std::process::exit(127)
}

/// The variable a shell inside a tab is marked with.
pub const INSIDE: &str = "TAB";

/// Whether this shell is running inside a tab, and which one.
pub fn current() -> Option<String> {
    std::env::var(INSIDE).ok().filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests;
