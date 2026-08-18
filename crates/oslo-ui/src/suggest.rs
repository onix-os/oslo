//! Ghost suggestions a config or a plugin supplies.
//!
//! ```lua
//! oslo.suggest.provider { name = "tldr", answer = function(ctx) return "…" end }
//! oslo.suggest.sh_sources = { "history", "provider", "predict", "path" }
//! ```
//!
//! # A source among the sources, not a replacement for them
//!
//! `provider` takes its place in `oslo.suggest.sh_sources` like the four built-in names, so where a
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
/// A predicate over the context: whether this provider is worth asking here at all.
pub type Enabled = Rc<dyn Fn(&Ctx) -> bool>;

pub type Answer = Rc<dyn Fn(&Ctx) -> Option<String>>;

/// Start work for `Ctx` that will answer later by calling [`answered`] with the same line.
///
/// **It must return promptly.** This runs on the keystroke path exactly like a synchronous answer;
/// what makes it asynchronous is what it *starts*, not how long it takes to start it. `oslo.spawn`
/// is the intended shape: hand a process off and answer from its `on_exit`.
pub type Request = Rc<dyn Fn(&Ctx)>;

/// What a late answer is allowed to do to what is already on screen.
///
/// **The decision this whole mechanism exists for.** `predict` answers in microseconds and a model
/// over a network answers in hundreds of milliseconds, so by the time the slow one speaks there is
/// usually already a suggestion drawn. Neither answer to "what now" is right for everybody, so it is
/// a setting rather than a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Late {
    /// Draw it only if nothing else did. Nothing on screen ever changes on its own.
    ///
    /// The default, and the reason it is the default: text moving under the cursor half a second
    /// after you stopped typing is startling, and startling is a bad default even when it is better.
    Fill,
    /// Draw it even over an answer another source already gave.
    ///
    /// What you ask for when the slow provider is the one you actually trust. Bounded by `settle`,
    /// so an answer that took long enough for you to have moved on does not rewrite the line anyway.
    Replace,
}

/// How a provider answers.
pub enum Ask {
    /// Now, on the keystroke path, within [`BUDGET`].
    Now(Answer),
    /// Later, by calling [`answered`]. The line is asked about only once typing has been still for
    /// `debounce`, and an answer that has not arrived within `timeout` is given up on.
    Later {
        request: Request,
        debounce: Duration,
        timeout: Duration,
        on_late: Late,
        /// How long an answer may take and still be allowed to replace what is drawn.
        ///
        /// Only [`Late::Replace`] reads it: filling a gap is never startling however long it took.
        settle: Duration,
    },
}

pub struct Provider {
    pub name: String,
    pub ask: Ask,
    /// Guards, checked before the provider is asked anything.
    pub only: Only,
}

/// When a provider is worth asking at all.
///
/// **Cheap tests first, and the predicate last.** `min_chars` and `max_line` are two integer
/// comparisons; `enabled` is a call into Lua, and calling it to find out that the line was two
/// characters long would be the expensive way to answer a cheap question.
pub struct Only {
    /// Do not ask about a line shorter than this. A model asked about `g` is being asked nothing.
    pub min_chars: usize,
    /// Do not ask about a line longer than this.
    ///
    /// From `ZSH_AUTOSUGGEST_BUFFER_MAX_SIZE`, and for its reason: a 4 KB line is a paste, not a
    /// prompt, and suggesting against it is expensive and pointless in the same breath.
    pub max_line: usize,
    /// Anything the two above cannot express — a predicate over the whole context.
    ///
    /// **This is the context rule.** A `when = { command = …, cwd = … }` table was the other design,
    /// and a Lua function is strictly more expressive than any table of fields would be, in the
    /// language the rest of the config is already written in. It is what says *not in this
    /// directory*, which for a provider that sends your typing somewhere is the setting that
    /// matters most.
    pub enabled: Option<Enabled>,
}

impl Default for Only {
    fn default() -> Self {
        Only {
            min_chars: 1,
            max_line: 512,
            enabled: None,
        }
    }
}

impl Only {
    fn asks(&self, ctx: &Ctx) -> bool {
        ctx.line.chars().count() >= self.min_chars
            && ctx.line.len() <= self.max_line
            && self.enabled.as_ref().is_none_or(|ask| ask(ctx))
    }
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
    /// What came back, and the line it was asked about.
    ///
    /// **Keyed by the line, which is what makes a stale answer impossible to draw.** A reply that
    /// arrives for `gi` after `git ` has been typed simply does not match, so it is never shown —
    /// rather than being dropped by a separate staleness check that could be forgotten.
    ///
    /// The inner `None` is a *remembered decline*: this line was asked about and the answer was
    /// nothing. Kept, because forgetting it would mean asking again on the very next frame, and
    /// again on the one after — a provider that declines would be asked for ever.
    answered: Option<(String, Option<String>)>,
    /// How long the answer took, for the `settle` bound.
    took: Duration,
    /// The line a request is out for, and when it went.
    in_flight: Option<(String, Instant)>,
    /// The line waiting for typing to go quiet, and the moment it stops waiting.
    waiting_on: Option<(String, Instant)>,
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
            answered: None,
            took: Duration::ZERO,
            in_flight: None,
            waiting_on: None,
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
    ask_in(ctx, Phase::Turn)
}

/// The second pass, after every other source declined: the answers that were told to fill a gap
/// rather than take one over.
///
/// **Asked outside the `sources` order on purpose.** A gap-filler has no position — it answers when
/// nothing else did, which is a different question from "answer before history".
pub fn ask_fill(ctx: &Ctx) -> Option<String> {
    ask_in(ctx, Phase::Fill)
}

/// Which pass this is: the provider source's turn in the order, or the gap-filling pass after it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Turn,
    Fill,
}

fn ask_in(ctx: &Ctx, phase: Phase) -> Option<String> {
    if !any() {
        return None;
    }
    // Decided before anything is called, and cloned out of the cell: an answer runs Lua, and Lua can
    // reach back into the shell — the same hazard `completion::config_candidates` documents, and the
    // same fix. `Step` is what each provider wants to happen, computed while the borrow is held.
    let steps = plan(ctx, phase);

    for (at, name, step) in steps {
        match step {
            Step::AnswerNow(answer) => {
                let started = Instant::now();
                let said = answer(ctx);
                note_time(at, &name, started.elapsed());
                let Some(whole) = said else { continue };
                match remainder(&ctx.line, &whole) {
                    Some(rest) => return Some(rest),
                    None => complain(at, &name, &ctx.line, &whole),
                }
            }
            // An answer already here for exactly this line.
            Step::Draw(whole) => match remainder(&ctx.line, &whole) {
                Some(rest) => return Some(rest),
                None => complain(at, &name, &ctx.line, &whole),
            },
            Step::Send(request) => {
                crate::pending::started();
                request(ctx);
            }
            // Waiting for the debounce, or for a reply. Neither draws anything.
            Step::Wait => {}
        }
    }
    None
}

/// What one provider wants to happen this frame.
enum Step {
    AnswerNow(Answer),
    Draw(String),
    Send(Request),
    Wait,
}

/// Decide every provider's step, and record the bookkeeping, in one borrow.
fn plan(ctx: &Ctx, phase: Phase) -> Vec<(usize, String, Step)> {
    let now = Instant::now();
    PROVIDERS.with(|slot| {
        let mut providers = slot.borrow_mut();
        let mut steps = Vec::new();
        for (at, entry) in providers.iter_mut().enumerate() {
            if entry.disabled || !entry.provider.only.asks(ctx) {
                continue;
            }
            let name = entry.provider.name.clone();
            let step = match &entry.provider.ask {
                // A synchronous answer is never late — it is given in the frame that asked — so it
                // has nothing to fill and belongs to the provider source's turn.
                Ask::Now(answer) => match phase {
                    Phase::Turn => Step::AnswerNow(Rc::clone(answer)),
                    Phase::Fill => Step::Wait,
                },
                Ask::Later {
                    request,
                    debounce,
                    timeout,
                    on_late,
                    settle,
                } => {
                    let (request, on_late, settle) = (Rc::clone(request), *on_late, *settle);
                    let (debounce, timeout) = (*debounce, *timeout);
                    // **The state machine runs in the turn pass only**, whichever mode this is: a
                    // gap-filler still has to send its request at the same moment, or it would ask
                    // only on the lines where everything else stayed silent.
                    let step = match phase {
                        Phase::Turn => later(entry, ctx, now, request, debounce, timeout),
                        Phase::Fill => ready(entry, ctx),
                    };
                    match (&step, phase, on_late) {
                        // Drawn in the other pass, not this one.
                        (Step::Draw(_), Phase::Turn, Late::Fill) => Step::Wait,
                        // An answer that took longer than it was allowed to may not take over what
                        // is already drawn. It is still there for the fill pass, where replacing
                        // nothing is not startling.
                        (Step::Draw(_), Phase::Turn, Late::Replace) if entry.took > settle => {
                            Step::Wait
                        }
                        _ => step,
                    }
                }
            };
            steps.push((at, name, step));
        }
        steps
    })
}

/// An answer already here for exactly this line, without touching the request state machine.
fn ready(entry: &Registered, ctx: &Ctx) -> Step {
    match &entry.answered {
        Some((line, Some(whole))) if line == &ctx.line => Step::Draw(whole.clone()),
        _ => Step::Wait,
    }
}

/// The state machine for one asynchronous provider: give up, draw, send, or wait.
fn later(
    entry: &mut Registered,
    ctx: &Ctx,
    now: Instant,
    request: Request,
    debounce: Duration,
    timeout: Duration,
) -> Step {
    // A reply that never came. Given up on so the editor stops waiting for it — an outstanding
    // answer that is never finished leaves the input loop polling for the rest of the session.
    if let Some((line, sent)) = entry.in_flight.clone()
        && now.duration_since(sent) >= timeout
    {
        entry.in_flight = None;
        // Remembered as a decline rather than simply forgotten, or the next frame would ask again
        // and the one after that — a provider that has stopped answering would be hammered at the
        // frame rate until the line changed.
        entry.answered = Some((line, None));
        crate::pending::finished();
    }

    // An answer for exactly this line. Not for a prefix of it and not for a longer line: those are
    // answers to different questions, and drawing one would be the stale-suggestion bug.
    if let Some((line, whole)) = &entry.answered
        && line == &ctx.line
    {
        return match whole {
            Some(whole) => Step::Draw(whole.clone()),
            None => Step::Wait,
        };
    }

    // Already asked about this line. Asking again would be a second request for the same answer.
    if entry
        .in_flight
        .as_ref()
        .is_some_and(|(line, _)| line == &ctx.line)
    {
        return Step::Wait;
    }

    // Start the wait, or start it again because the line changed — the old wait was for a question
    // nobody is asking any more.
    let restart = entry
        .waiting_on
        .as_ref()
        .is_none_or(|(line, _)| line != &ctx.line);
    if restart {
        entry.waiting_on = Some((ctx.line.clone(), now + debounce));
    }

    // **Checked after being set, not on the next frame.** Setting the deadline and returning would
    // mean a `debounce` of zero — which is what a provider asks for when it wants no delay at all —
    // never firing until something else caused a redraw.
    let Some((_, until)) = entry.waiting_on else {
        return Step::Wait;
    };
    if now < until {
        crate::pending::wake_in(until - now);
        return Step::Wait;
    }
    entry.waiting_on = None;
    entry.in_flight = Some((ctx.line.clone(), now));
    Step::Send(request)
}

/// A provider answering the request it was given, from wherever its work finished.
///
/// `line` is the line it was *asked* about, not the one on screen now. The two are compared when the
/// answer is drawn, which is what makes a late reply harmless rather than wrong.
pub fn answered(name: &str, line: &str, whole: Option<String>) {
    PROVIDERS.with(|slot| {
        let mut providers = slot.borrow_mut();
        let Some(entry) = providers
            .iter_mut()
            .find(|entry| entry.provider.name == name)
        else {
            return;
        };
        // Only if this is the reply to the outstanding request. A provider that replies twice, or
        // replies to something it was never asked, does not get to unbalance the wait counter.
        if entry
            .in_flight
            .as_ref()
            .is_none_or(|(sent, _)| sent != line)
        {
            return;
        }
        entry.took = entry
            .in_flight
            .as_ref()
            .map(|(_, sent)| sent.elapsed())
            .unwrap_or_default();
        entry.in_flight = None;
        entry.answered = Some((line.to_string(), whole));
        crate::pending::finished();
        // The frame is stale: something can be drawn now that could not a moment ago.
        crate::pending::landed();
    });
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
