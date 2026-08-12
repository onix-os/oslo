//! Ghost suggestions a config or a plugin supplies.
//!
//! ```lua
//! oslo.suggest.provider { name = "tldr", answer = function(ctx) return "…" end }
//! oslo.suggest.sources = { "history", "provider", "predict", "path" }
//! ```
//!
//! # A source among the sources, not a replacement for them
//!
//! `provider` takes its place in `oslo.suggest.sources` like the four built-in names, so where a
//! plugin's answer sits relative to your own history is one line of config and it is *your* line.
//! VS Code has the same mechanism the other way round — `yieldsToGroupIds` is declared by the
//! provider, so the plugin decides whether it defers to you. That is the thing worth not copying.
//!
//! # The continuation invariant
//!
//! **A ghost is drawn as trailing text and accepted with Right.** So an answer that is not a
//! continuation of what is on the line would make that key insert something that was never
//! suggested. Such an answer is refused and reported — never trimmed into something the provider
//! did not write, which would put words in its mouth and hide the bug.
//!
//! A provider that wants to *replace* the line is asking for the repair slot, which is the model's.
//!
//! # The budget
//!
//! This runs on the keystroke path. A provider that takes 50 ms makes the shell feel broken, and
//! from the outside it looks like oslo being slow rather than like a plugin being slow. One that
//! overruns [`BUDGET`] repeatedly is switched off for the session and says which it was.
//!
//! Thread-local, because only the editor's thread suggests anything and an answer is Lua.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// What a provider is told about the line being typed.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// The line as typed, up to the cursor — which is the end of the line, or there is no ghost.
    pub line: String,
    pub cursor: usize,
    pub cwd: String,
    /// `sh` or `lua`.
    pub language: String,
}

/// A provider's answer: the **whole line** it thinks you are typing, or nothing.
///
/// The whole line rather than the remainder, because that is what a provider naturally has and
/// because the invariant is checkable only against the whole thing. The remainder is computed here.
pub type Answer = Rc<dyn Fn(&Ctx) -> Option<String>>;

pub struct Provider {
    pub name: String,
    pub answer: Answer,
}

/// How long one synchronous provider may take before it is a problem.
///
/// A keystroke's whole budget is a few milliseconds — `bench/keystroke.rs` measures the repaint at
/// well under one — so 20 ms is already an eternity and is chosen to be a threshold nobody hits by
/// accident. A provider that wants longer is asking to be asynchronous.
pub const BUDGET: Duration = Duration::from_millis(20);

/// How many overruns are forgiven. A cold page cache, a first call that compiles a regex: one slow
/// answer is not a slow provider, and disabling on the first would be a coin toss.
const FORGIVEN: u32 = 3;

struct Registered {
    provider: Provider,
    overruns: u32,
    /// Set once it has been switched off, so the report is written once rather than per keystroke.
    disabled: bool,
    /// Set once it has answered with something that was not a continuation, for the same reason.
    complained: bool,
}

thread_local! {
    static PROVIDERS: RefCell<Vec<Registered>> = const { RefCell::new(Vec::new()) };
    /// Whether anything is registered, without borrowing the list to find out.
    ///
    /// **Thread-local, like the list it describes.** It was a process-wide atomic, which is wrong in
    /// a way that only shows up under a test runner: one thread calling `forget` cleared the flag
    /// for every other thread, whose providers were still registered and silently stopped being
    /// asked. Production has one editor thread and would never have shown it.
    static ANY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether anything is registered at all, without touching the list.
///
/// **The keystroke path asks this first.** A shell with no providers — which is every shell until
/// somebody installs one — must not pay for the mechanism, and a flag read is a cost that does not
/// show up in a measurement.
pub fn any() -> bool {
    ANY.with(|any| any.get())
}

/// Add a provider, replacing any earlier one of the same name.
///
/// Replacing rather than refusing, so a config that is edited and re-sourced does not keep the first
/// version of everything it declared.
pub fn register(provider: Provider) {
    PROVIDERS.with(|slot| {
        let mut providers = slot.borrow_mut();
        let entry = Registered {
            provider,
            overruns: 0,
            disabled: false,
            complained: false,
        };
        match providers
            .iter()
            .position(|p| p.provider.name == entry.provider.name)
        {
            Some(at) => providers[at] = entry,
            None => providers.push(entry),
        }
    });
    ANY.with(|any| any.set(true));
}

/// Forget everything registered. For tests, and for a config being reloaded.
pub fn forget() {
    PROVIDERS.with(|slot| slot.borrow_mut().clear());
    ANY.with(|any| any.set(false));
}

/// Every provider's name, in the order they are asked.
pub fn names() -> Vec<String> {
    PROVIDERS.with(|slot| {
        slot.borrow()
            .iter()
            .map(|p| p.provider.name.clone())
            .collect()
    })
}

/// Ask the providers in turn; answer the **remainder** to draw after the cursor.
///
/// The first one with a usable answer wins, which is how the built-in sources behave among
/// themselves. `None` means every provider declined, and the next source is asked.
pub fn ask(ctx: &Ctx) -> Option<String> {
    if !any() {
        return None;
    }
    // Cloned out of the cell before calling: an answer runs Lua, and Lua can reach back into the
    // shell — the same hazard `completion::config_candidates` documents, and the same fix.
    let ready: Vec<(usize, String, Answer)> = PROVIDERS.with(|slot| {
        slot.borrow()
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.disabled)
            .map(|(at, p)| (at, p.provider.name.clone(), Rc::clone(&p.provider.answer)))
            .collect()
    });

    for (at, name, answer) in ready {
        let started = Instant::now();
        let said = answer(ctx);
        note_time(at, &name, started.elapsed());
        let Some(whole) = said else { continue };
        match remainder(&ctx.line, &whole) {
            Some(rest) => return Some(rest),
            None => complain(at, &name, &ctx.line, &whole),
        }
    }
    None
}

/// The part of `whole` that comes after `line`, if `whole` really continues it.
///
/// Equal is not a continuation: there is nothing to draw, and offering an empty ghost would light up
/// the accept keys for a suggestion that adds nothing.
fn remainder(line: &str, whole: &str) -> Option<String> {
    whole
        .strip_prefix(line)
        .filter(|rest| !rest.is_empty())
        .map(str::to_string)
}

fn note_time(at: usize, name: &str, took: Duration) {
    if took <= BUDGET {
        return;
    }
    PROVIDERS.with(|slot| {
        let mut providers = slot.borrow_mut();
        let Some(entry) = providers.get_mut(at) else {
            return;
        };
        entry.overruns += 1;
        if entry.overruns <= FORGIVEN {
            return;
        }
        entry.disabled = true;
        oslo_base::messages::warn(
            format!("suggest/{name}"),
            format!(
                "took {} ms on the keystroke path more than {FORGIVEN} times, so it is switched \
                 off for this session; a provider that needs longer wants `request`, not `answer`",
                took.as_millis()
            ),
        );
    });
}

fn complain(at: usize, name: &str, line: &str, whole: &str) {
    PROVIDERS.with(|slot| {
        let mut providers = slot.borrow_mut();
        let Some(entry) = providers.get_mut(at) else {
            return;
        };
        if entry.complained {
            return;
        }
        entry.complained = true;
        oslo_base::messages::warn(
            format!("suggest/{name}"),
            format!(
                "answered {whole:?} for {line:?}, which does not continue it — a ghost is drawn \
                 after what you typed and accepted with Right, so it can only ever be a \
                 continuation. Nothing was drawn."
            ),
        );
    });
}

#[cfg(test)]
#[path = "suggest/tests.rs"]
mod tests;
