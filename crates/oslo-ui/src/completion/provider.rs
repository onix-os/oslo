//! Dropdown candidates a config or a plugin supplies.
//!
//! ```lua
//! oslo.completion.provider {
//!   name = "tldr", kind = "example", score_offset = 20,
//!   answer = function(ctx) return { { display = "git commit --amend", description = "…" } } end,
//! }
//! ```
//!
//! # It adds, where `for_command` replaces
//!
//! `oslo.completion.for_command.git` means *I own git* — oslo's own candidates for that command are
//! dropped. That is the right tool for a command whose completions you are rewriting, and the wrong
//! one for tldr, which wants to put three examples *next to* the subcommands oslo already knows.
//!
//! So a provider merges. Which is also why it needs the two things `for_command` never had:
//!
//! - **a kind**, so `oslo.completion.sources` can name it and the badge column can show it. A
//!   `for_command` candidate used to report no kind at all, which meant setting `sources` silently
//!   removed every config-supplied candidate; it now carries `config`, so both paths are
//!   filterable, and a provider's is the more specific of the two.
//! - **a score offset**, because merging means competing. The dropdown already sorts by frecency,
//!   and an offset is added to that score rather than replacing the order — blink.cmp's `score_offset`
//!   rather than a priority that overrules everything.
//!
//! # Every command, or one
//!
//! `for_command` is a table keyed by command name, so a provider that wants to answer for *anything*
//! would have to enumerate every command on the machine. `when` is a name or nothing at all.

use super::CompletionCandidate;
use std::cell::RefCell;
use std::rc::Rc;

/// What a provider is told about the word being completed.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// The command being completed for — the first word, after aliases are followed.
    pub command: String,
    /// The words already typed, `command` first.
    pub words: Vec<String>,
    /// The partial word the dropdown is completing.
    pub current: String,
    /// Which argument this is: 1 for the first word after the command.
    pub arg: usize,
    pub cwd: String,
}

/// One offer. `display` is what is shown and inserted; the description is the second column.
#[derive(Debug, Clone)]
pub struct Offer {
    pub display: String,
    pub description: Option<String>,
}

/// A predicate over the context: whether this provider is worth asking here at all.
pub type Enabled = Rc<dyn Fn(&Ctx) -> bool>;

pub type Answer = Rc<dyn Fn(&Ctx) -> Vec<Offer>>;

pub struct Provider {
    pub name: String,
    /// The badge, and the name `oslo.completion.sources` filters on. A provider that declares none
    /// gets its own name, which says more than the flat `config` a `for_command` candidate carries.
    pub kind: String,
    /// Only for this command, or for every one.
    pub when: Option<String>,
    /// Added to the frecency score when the list is sorted.
    pub score_offset: f64,
    /// At most this many offers are taken, so one provider cannot flood the menu.
    pub max_items: usize,
    /// Do not ask until this much of the word has been typed.
    pub min_chars: usize,
    /// Anything the other fields cannot express — a predicate over the whole context.
    ///
    /// The same idea as the ghost's, and the same reason: a Lua function is more expressive than any
    /// table of `when` fields, in the language the config is already written in.
    pub enabled: Option<Enabled>,
    pub answer: Answer,
}

/// The default cap. Generous enough that a real answer is never truncated, small enough that a
/// provider returning its entire database does not push everything else off the screen.
pub const DEFAULT_MAX_ITEMS: usize = 50;

thread_local! {
    static PROVIDERS: RefCell<Vec<Provider>> = const { RefCell::new(Vec::new()) };
    /// Whether anything is registered, without borrowing the list. Read once per Tab.
    static ANY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn any() -> bool {
    ANY.with(|any| any.get())
}

/// Add a provider, replacing any earlier one of the same name.
pub fn register(provider: Provider) {
    PROVIDERS.with(|slot| {
        let mut providers = slot.borrow_mut();
        match providers.iter().position(|p| p.name == provider.name) {
            Some(at) => providers[at] = provider,
            None => providers.push(provider),
        }
    });
    ANY.with(|any| any.set(true));
}

pub fn forget() {
    PROVIDERS.with(|slot| slot.borrow_mut().clear());
    ANY.with(|any| any.set(false));
}

/// Drop the provider called `name`, answering whether there was one.
///
/// **`ANY` is re-set from the list rather than left alone.** It is the flag every Tab press checks
/// before paying for a walk; leaving it true once the last provider has gone means the cost stays
/// after the reason for it is over.
pub fn forget_named(name: &str) -> bool {
    PROVIDERS.with(|slot| {
        let mut providers = slot.borrow_mut();
        let before = providers.len();
        providers.retain(|provider| provider.name != name);
        let removed = providers.len() != before;
        ANY.with(|any| any.set(!providers.is_empty()));
        removed
    })
}

pub fn names() -> Vec<String> {
    PROVIDERS.with(|slot| slot.borrow().iter().map(|p| p.name.clone()).collect())
}

/// Everything the providers offer for this context, ready to merge into the candidate list.
///
/// Answers the score offset alongside each candidate, because the sort happens later and over the
/// whole merged list — a provider's offer competes with oslo's own rather than being placed above or
/// below them wholesale.
pub fn offers(ctx: &Ctx) -> Vec<(CompletionCandidate, f64)> {
    if !any() {
        return Vec::new();
    }
    // Cloned out of the cell before calling: an answer runs Lua, and Lua can complete another word,
    // which would come back through here and panic on the outstanding borrow. The same hazard
    // `config_candidates` documents.
    let ready: Vec<(String, String, f64, usize, Answer)> = PROVIDERS.with(|slot| {
        slot.borrow()
            .iter()
            .filter(|p| p.when.as_ref().is_none_or(|only| only == &ctx.command))
            .filter(|p| ctx.current.chars().count() >= p.min_chars)
            .filter(|p| p.enabled.as_ref().is_none_or(|ask| ask(ctx)))
            .map(|p| {
                (
                    p.name.clone(),
                    p.kind.clone(),
                    p.score_offset,
                    p.max_items,
                    Rc::clone(&p.answer),
                )
            })
            .collect()
    });

    let mut out = Vec::new();
    for (_name, kind, offset, max_items, answer) in ready {
        for offer in answer(ctx).into_iter().take(max_items) {
            if offer.display.is_empty() {
                continue;
            }
            out.push((
                CompletionCandidate {
                    replacement: offer.display.clone(),
                    display: offer.display,
                    description: offer.description,
                    kind: Some(kind.clone()),
                    path: None,
                    detail: None,
                },
                offset,
            ));
        }
    }
    out
}

#[cfg(test)]
#[path = "provider/tests.rs"]
mod tests;
