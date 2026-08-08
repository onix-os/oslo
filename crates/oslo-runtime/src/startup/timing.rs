//! Where a prompt's time goes, when someone asks.
//!
//! A shell that feels slow between commands is a shell nobody can profile from the outside: the
//! work is spread across a Lua prompt function, a directory check, a hook that may shell out, and
//! a terminal negotiation, and `strace` shows all of it at once with no idea which phase asked for
//! what. This attributes the wall-clock to named phases, so the answer is read off rather than
//! guessed at — which is the only way anyone has ever found one of these.
//!
//! Off unless `$OSLO_TIME_PROMPT` is set, and the check is one `getenv` per prompt cached into a
//! bool, so a shell that is not being profiled pays nothing.
//!
//! ```text
//! $ OSLO_TIME_PROMPT=1 oslo
//! oslo: prompt 41.6ms — read 38.9 · precmd 1.9 · direnv 0.5 · size 0.2 · universal 0.1
//! ```

use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Whether to measure at all.
fn wanted() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("OSLO_TIME_PROMPT").is_some())
}

thread_local! {
    /// This prompt's phases, in the order they were first seen.
    static PHASES: RefCell<Vec<(&'static str, Duration)>> = const { RefCell::new(Vec::new()) };
}

/// Time `f`, recording it against `phase`.
///
/// Nested calls are not subtracted from their caller: every phase here is a sibling, and one that
/// grows to contain another should be split rather than nested.
pub fn phase<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    if !wanted() {
        return f();
    }
    let started = Instant::now();
    let value = f();
    let took = started.elapsed();
    PHASES.with(|phases| {
        let mut phases = phases.borrow_mut();
        match phases.iter_mut().find(|(seen, _)| *seen == name) {
            Some((_, total)) => *total += took,
            None => phases.push((name, took)),
        }
    });
    value
}

/// [`phase`] for a body that cannot be wrapped in a closure, ending when the guard drops.
pub fn open(name: &'static str) -> Open {
    Open {
        name,
        started: wanted().then(Instant::now),
    }
}

pub struct Open {
    name: &'static str,
    started: Option<Instant>,
}

impl Drop for Open {
    fn drop(&mut self) {
        let Some(started) = self.started else {
            return;
        };
        let took = started.elapsed();
        PHASES.with(|phases| {
            let mut phases = phases.borrow_mut();
            match phases.iter_mut().find(|(seen, _)| *seen == self.name) {
                Some((_, total)) => *total += took,
                None => phases.push((self.name, took)),
            }
        });
    }
}

/// Report what this prompt cost and start the next one, slowest phase first.
///
/// Written to stderr, so a shell being profiled while its stdout is redirected still reports.
pub fn report() {
    if !wanted() {
        return;
    }
    let mut phases = PHASES.with(|phases| std::mem::take(&mut *phases.borrow_mut()));
    if phases.is_empty() {
        return;
    }
    let total: Duration = phases.iter().map(|(_, took)| *took).sum();
    phases.sort_by_key(|(_, took)| std::cmp::Reverse(*took));
    let detail: Vec<String> = phases
        .iter()
        .map(|(name, took)| format!("{name} {:.1}", took.as_secs_f64() * 1000.0))
        .collect();
    eprintln!(
        "oslo: prompt {:.1}ms — {}",
        total.as_secs_f64() * 1000.0,
        detail.join(" · ")
    );
}
