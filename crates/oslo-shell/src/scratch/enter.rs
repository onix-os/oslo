//! The key, the finder it opens, and what is done with the answer.
//!
//! This is the whole of the feature that anybody sees. Everything else in this module is the
//! machinery it stands on.
//!
//! # One key, two places, one meaning
//!
//! Outside a scratch the shell owns the terminal, so the key is an ordinary editor binding and this is
//! called from a prompt. Inside a scratch the client owns it and the shell is on the far side of a pty,
//! so the client takes the key out of the stream and calls the same thing. The finder does not know
//! which happened; only what Esc means differs, and that is the caller's to say.
//!
//! ```text
//!            ┌── pick an existing one ──► attach to it
//!   ^\ ──►  finder ── type a name ──────► make it, attach
//!            └── Esc ──────────────────► outside: nothing. inside: leave the scratch running.
//! ```
//!
//! # Why the shell inside is `exec`d rather than forked on
//!
//! `spawn` hands the child a choice: carry on as the shell it already is, or become a new one. Here
//! it must become a new one. The key is pressed at a prompt, long after oslo has started the
//! threads that warm the `$PATH` index — and `fork` carries only the calling thread, so a child
//! that carried on would hold locks belonging to threads that do not exist in it. `exec` costs one
//! more config read, once, when a scratch is made; that is the right price for not shipping a deadlock
//! that appears under load.

use super::{backend, client, detach, dir, keeper, name as naming};
use oslo_ui::ask::{Answer, Choice, Pick, pick_or_create};
use std::io::{self, IsTerminal};

/// What the key did, so a caller can tell "nothing happened" from "you are somewhere else now".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Went {
    /// The finder was dismissed. Nothing was made, entered or left.
    Nowhere,
    /// A scratch was entered and has since been left, or ended.
    ThereAndBack,
}

/// Open the finder and act on the answer, until nothing is left to attach to.
///
/// **Returns only when the terminal is the caller's again**, so a prompt can be redrawn and a
/// client can exit. Everything in between — attaching, the key being pressed again inside, moving
/// to another scratch — happens in the loop.
pub fn open(key: &str, replay: u64) -> io::Result<Went> {
    // **Before anything reads or writes there.** Listing touches the directory and so does asking
    // whether a name is alive — which creates its lock file. A check that ran later would have
    // already left a file in a directory it was about to refuse.
    dir::open_checked()?;
    // Read once, here, so a session cannot end up half in one backend and half in the other
    // because the setting changed while a scratch was attached.
    let scratches = backend::current(oslo_ui::settings::current().scratch.daemon);
    let detach = detach::Key::named(key);
    let mut went = Went::Nowhere;
    let mut next = ask(&*scratches, None)?;

    while let Some(name) = next {
        scratches.ensure(&name)?;
        went = Went::ThereAndBack;
        match client::attach(scratches.connect(&name)?, &name, detach, replay)? {
            // The shell inside exited, so there is nothing to go back to and nothing to ask about.
            client::Left::Ended => return Ok(went),
            // The key, pressed inside. The finder opens on the terminal the client has just handed
            // back, and Esc here means leave — which is the one thing it cannot mean outside.
            client::Left::Detached => next = ask(&*scratches, Some(&name))?,
        }
    }
    Ok(went)
}

/// Every scratch there is, without asking anything.
///
/// Separate from the finder because the caller is a prompt or a script, and a widget is the wrong
/// answer to "how many are running".
pub fn names() -> io::Result<Vec<String>> {
    dir::open_checked()?;
    backend::current(oslo_ui::settings::current().scratch.daemon).list()
}

/// Go straight into `name`, making it if nothing is holding it.
///
/// The finder still opens if the key is pressed inside, so this is a way *in* rather than a second
/// way of choosing: what happens after you arrive is the same either way.
pub fn open_named(key: &str, replay: u64, name: &str) -> io::Result<Went> {
    // **Before the session is made, not after the attach fails.** A scratch is a terminal you go
    // into, and `ensure` forks the keeper and execs the shell inside it — so a `scratch work` from
    // a script or a pipeline used to build the whole session, discover at the attach that there is
    // no terminal, report the failure and exit 1, and leave the shell it had just started running
    // with nobody attached and nobody able to attach. One per invocation, until the machine was
    // rebooted or somebody went looking for them.
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "a scratch is a terminal to go into, and there is none here",
        ));
    }
    if !naming::valid(name) {
        return Err(io::Error::other(format!(
            "{name:?} is not a usable scratch name"
        )));
    }
    dir::open_checked()?;
    let scratches = backend::current(oslo_ui::settings::current().scratch.daemon);
    let detach = detach::Key::named(key);
    let mut next = Some(name.to_string());

    while let Some(name) = next {
        scratches.ensure(&name)?;
        match client::attach(scratches.connect(&name)?, &name, detach, replay)? {
            client::Left::Ended => return Ok(Went::ThereAndBack),
            client::Left::Detached => next = ask(&*scratches, Some(&name))?,
        }
    }
    Ok(Went::ThereAndBack)
}

/// The finder, listing every scratch and offering what is typed as a new one.
///
/// `inside` is the scratch the question is being asked from, which **is listed like any other**: going
/// back into the one you are in has to be possible, or the key is a trap for anybody who pressed it
/// by accident.
fn ask(scratches: &dyn backend::Scratches, inside: Option<&str>) -> io::Result<Option<String>> {
    let running = scratches.list()?;
    let spec = Choice {
        items: running.clone(),
        look: look(inside),
        // Short on purpose. This is a list of sessions, not of history — there are as many rows as
        // you have scratches, and a panel sized for a thousand lines would be mostly empty air.
        height: 8,
        chrome: oslo_ui::ask::chrome::Chrome {
            // **No legend.** It lists three keys, two of which are the ones every list in the shell
            // already uses; against four names it is more rows of explanation than of answer. The
            // box keeps its width without it — see `look`.
            legend: false,
            legend_gap: 0,
            ..oslo_ui::ask::chrome::Chrome::default()
        },
        ..Choice::default()
    };

    Ok(match pick_or_create(&spec, "{}") {
        Answer::Given(Pick::Chosen(name)) => Some(name),
        // Refused rather than quietly rewritten: a name is part of a path, and a name you did not
        // type is one you cannot find again. See `name::valid`.
        Answer::Given(Pick::New(typed)) if !naming::valid(&typed) => {
            return Err(io::Error::other(format!(
                "{typed:?} is not a usable scratch name"
            )));
        }
        Answer::Given(Pick::New(typed)) => Some(typed),
        // Esc. Outside, the caller has nothing to do; inside, this is how you leave.
        Answer::Cancelled => None,
        // No terminal is not a refusal to answer, it is a place the question cannot be asked.
        Answer::NoTerminal => None,
    })
}
/// tab-rs's finder, in oslo's colours.
///
/// The shape is copied deliberately, because it is the one that reads as a *search* rather than as
/// a list with a label on it: the query on the first row where the cursor already is, what it found
/// on the second, and the names below with nothing drawn around them.
///
/// ```text
///   >>  ap            what you are typing
///       1/3           what it found
///     work
///   >   api           the one Enter takes
/// ```
///
/// What is oslo's rather than tab-rs's is every colour — the accent on the prompt and the marker,
/// the muted grey on the count, the underline on a matched run — so this follows the theme like
/// everything else instead of hard-coding blue as tab-rs does.
fn look(_inside: Option<&str>) -> oslo_ui::ask::look::Look {
    use oslo_ui::ask::look::{Where, Width};
    use oslo_ui::theme::Style;
    let theme = oslo_ui::theme::current();
    let pager = &theme.pager;

    oslo_ui::ask::look::Look {
        // Top, and not reversed: the list reads downward from what you typed.
        filter_at: Where::Top,
        reverse: false,
        prompt: ">>  ".to_string(),
        placeholder: "type to filter, or a name for a new one".to_string(),
        // `1/3` under the query, as tab-rs puts it — below the box rather than inside it, so the
        // tint is the thing you type in and nothing else.
        under: "    {n}/{total}".to_string(),
        // The box is back, and at the top: a tinted row above and below the query is what makes it
        // read as somewhere to type.
        surface: pager.bg,
        surface_rows: 3,
        // The history finder's list, down to the quiet grey on every other row. A stripe is a
        // ruler across a full-width row — which is why it comes with `Width::Full` and not without:
        // painted only as far as the text, it reads as a highlighted word rather than a rule.
        stripe: Some(Style {
            bg: Some(oslo_ui::theme::Color::Indexed(235)),
            ..Style::default()
        }),
        selected: Style {
            bg: pager.sel_bg,
            ..pager.text_sel
        },
        marker: "> ".to_string(),
        width: Width::Full,
        // One column each side. `Chrome` already keeps one at the right edge, and a block flush
        // against the left with a gap at the right reads as one that failed to reach the edge.
        margin: 1,
        gap: 0,
        pad: 1,
        scanner: None,
        left: String::new(),
        right: String::new(),
        badge: String::new(),
        // A matched run is underlined rather than painted, which is tab-rs's mark and the one that
        // survives a name that is mostly match.
        hit: Style {
            underline: true,
            ..Style::default()
        },
        ..oslo_ui::ask::look::Look::default()
    }
}

/// Carry on as the caller, or — in the process that turned out to be the shell — become one.
pub(super) fn become_shell_or(role: keeper::Role, name: &str) -> io::Result<()> {
    match role {
        keeper::Role::Caller(_) => Ok(()),
        keeper::Role::Inside => exec_inside(name),
    }
}

/// Become the shell inside a scratch. Never returns.
///
/// The environment is marked here rather than by the caller, so every path into a scratch agrees on
/// what `$SCRATCH` says.
pub(super) fn exec_inside(name: &str) -> ! {
    // SAFETY: a fresh fork about to `exec` — single-threaded, and nothing else can be reading the
    // environment while it is written.
    unsafe { std::env::set_var(INSIDE, name) };
    // `/proc/self/exe` rather than `$SHELL` or a name on `$PATH`: a scratch is an oslo, and the one
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

/// The variable a shell inside a scratch is marked with.
pub const INSIDE: &str = "SCRATCH";

/// Whether this shell is running inside a scratch, and which one.
pub fn current() -> Option<String> {
    std::env::var(INSIDE).ok().filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests;
