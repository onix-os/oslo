//! What a provider that answers *later* is allowed to do, and what the wait costs.
//!
//! Split from [`super`] when that file crossed the line limit. The division is the one the sections
//! already drew: above, a provider answers in the frame it is asked; here, it answers in some later
//! frame, and everything hard about suggestions lives in that gap — the debounce, the timeout, the
//! stale answer, and the counter that decides whether the editor may block on a key.

use super::{ctx, saying, serialised, slow, slow_with};
use crate::suggest::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

// ---------------------------------------------------------------- what a late answer may do

/// **`fill` never changes what is drawn.** It answers only in the second pass, which runs when every
/// source declined — so a provider set this way can add a suggestion but never take one over.
#[test]
fn a_gap_filler_is_silent_in_its_own_turn() {
    let _serial = serialised();
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow_with(
        "llm",
        Rc::clone(&asked),
        Duration::ZERO,
        Late::Fill,
        Duration::from_secs(5),
    ));

    ask(&ctx("git s"));
    answered("llm", "git s", Some("git status".to_string()));

    assert_eq!(ask(&ctx("git s")), None, "not in the provider's turn");
    assert_eq!(
        ask_fill(&ctx("git s")),
        Some("tatus".to_string()),
        "but it fills the gap when nothing else answered"
    );
    forget();
}

/// `replace` answers in its own turn, which is what puts it ahead of the sources listed after it.
#[test]
fn a_replacing_provider_answers_in_its_turn() {
    let _serial = serialised();
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow_with(
        "llm",
        Rc::clone(&asked),
        Duration::ZERO,
        Late::Replace,
        Duration::from_secs(5),
    ));

    ask(&ctx("git s"));
    answered("llm", "git s", Some("git status".to_string()));
    assert_eq!(ask(&ctx("git s")), Some("tatus".to_string()));
    forget();
}

/// **What makes `replace` liveable.** An answer that took longer than `settle` may not rewrite the
/// line under you — but it is still worth having where there is nothing to rewrite.
#[test]
fn an_answer_that_took_too_long_may_still_fill_but_not_replace() {
    let _serial = serialised();
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow_with(
        "llm",
        Rc::clone(&asked),
        Duration::ZERO,
        Late::Replace,
        Duration::from_millis(10),
    ));

    ask(&ctx("git s"));
    std::thread::sleep(Duration::from_millis(15));
    answered("llm", "git s", Some("git status".to_string()));

    assert_eq!(ask(&ctx("git s")), None, "too late to take anything over");
    assert_eq!(
        ask_fill(&ctx("git s")),
        Some("tatus".to_string()),
        "and still offered where nothing else answered"
    );
    forget();
}

/// A gap-filler still sends its request at the ordinary moment; only the drawing is deferred.
#[test]
fn a_gap_filler_is_still_asked_on_time() {
    let _serial = serialised();
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow_with(
        "llm",
        Rc::clone(&asked),
        Duration::ZERO,
        Late::Fill,
        Duration::from_secs(5),
    ));

    ask(&ctx("git s"));
    assert_eq!(asked.borrow().clone(), vec!["git s".to_string()]);
    forget();
}

/// The fill pass must not drive the state machine, or a provider would be asked twice per frame.
#[test]
fn the_fill_pass_asks_nothing_new() {
    let _serial = serialised();
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow_with(
        "llm",
        Rc::clone(&asked),
        Duration::ZERO,
        Late::Fill,
        Duration::from_secs(5),
    ));

    ask(&ctx("git s"));
    ask_fill(&ctx("git s"));
    assert_eq!(asked.borrow().len(), 1, "one frame, one request");
    forget();
}

// ---------------------------------------------------------------- when to ask at all

fn guarded(name: &str, only: Only) -> (Provider, Rc<RefCell<Vec<String>>>) {
    let asked = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&asked);
    (
        Provider {
            name: name.to_string(),
            ask: Ask::Now(Rc::new(move |ctx| {
                seen.borrow_mut().push(ctx.line.clone());
                Some(format!("{} --done", ctx.line))
            })),
            only,
        },
        asked,
    )
}

/// A model asked about `g` is being asked nothing.
#[test]
fn a_line_shorter_than_min_chars_is_not_worth_asking_about() {
    let _serial = serialised();
    forget();
    let (provider, asked) = guarded(
        "llm",
        Only {
            min_chars: 3,
            ..Only::default()
        },
    );
    register(provider);

    assert_eq!(ask(&ctx("gi")), None);
    assert!(asked.borrow().is_empty());
    assert!(ask(&ctx("git")).is_some());
    forget();
}

/// **A pasted line is not a prompt.** From `ZSH_AUTOSUGGEST_BUFFER_MAX_SIZE`, for its reason.
#[test]
fn a_pasted_line_is_too_long_to_suggest_against() {
    let _serial = serialised();
    forget();
    let (provider, asked) = guarded(
        "llm",
        Only {
            max_line: 16,
            ..Only::default()
        },
    );
    register(provider);

    assert_eq!(ask(&ctx(&"x".repeat(64))), None);
    assert!(asked.borrow().is_empty());
    forget();
}

/// **The context rule.** A predicate over the whole context is what says *not in this directory* —
/// which for a provider that sends your typing somewhere is the setting that matters most.
#[test]
fn a_predicate_decides_anything_the_others_cannot() {
    let _serial = serialised();
    forget();
    let (provider, asked) = guarded(
        "llm",
        Only {
            enabled: Some(Rc::new(|ctx| ctx.cwd != "/w/private")),
            ..Only::default()
        },
    );
    register(provider);

    assert!(ask(&ctx("git s")).is_some());
    let mut secret = ctx("git s");
    secret.cwd = "/w/private".to_string();
    assert_eq!(ask(&secret), None);
    assert_eq!(asked.borrow().len(), 1, "and it was not even asked");
    forget();
}

/// **A predicate that touches the registry does not abort the shell.**
///
/// `enabled` is arbitrary Lua and used to be evaluated inside `plan`'s `borrow_mut` on the provider
/// list — under the very comment warning that an answer must not be. A predicate that registered
/// another provider, which is the natural "offer this once the plugin has loaded" idiom, reached
/// `register`'s `borrow_mut` while the outer borrow was live and took the shell down with a
/// `BorrowMutError` on a keystroke. Even a read-only `providers()` from a predicate was a
/// `BorrowError`.
#[test]
fn a_predicate_may_ask_about_the_registry_it_is_deciding_about() {
    let _serial = serialised();
    forget();

    // Read-only re-entry: the shape a predicate checking "am I already registered?" has.
    let only = Only {
        enabled: Some(Rc::new(|_| {
            let _ = names();
            true
        })),
        ..Only::default()
    };
    register(Provider {
        name: "reader".to_string(),
        ask: Ask::Now(Rc::new(|_| Some("git status".to_string()))),
        only,
    });
    assert_eq!(ask(&ctx("git ")), Some("status".to_string()));

    // And mutating re-entry: registering from inside a predicate.
    forget();
    let only = Only {
        enabled: Some(Rc::new(|_| {
            // Registered once; a second call would find the name already there.
            if !names().iter().any(|name| name == "late") {
                register(saying("late", Some("git log")));
            }
            true
        })),
        ..Only::default()
    };
    register(Provider {
        name: "opener".to_string(),
        ask: Ask::Now(Rc::new(|_| None)),
        only,
    });
    // The point is that this returns at all rather than aborting.
    let _ = ask(&ctx("git "));
    assert!(
        names().iter().any(|name| name == "late"),
        "the predicate's registration took effect: {:?}",
        names()
    );
    forget();
}

/// **A superseded request does not leave the counter above zero.**
///
/// `pending::outstanding()` is what the editor reads to decide whether an answer may still arrive:
/// while it is true the input loop polls instead of blocking on a key. Overwriting `in_flight`
/// abandoned the outstanding request without decrementing, and neither balancing path could recover
/// it — the timeout sweep inspects only the current request, and `answered` returns early once the
/// line no longer matches. So typing, pausing, typing again left the prompt spinning at the frame
/// rate for the rest of the session, drifting further up with every abandoned request.
#[test]
fn a_superseded_request_is_not_left_outstanding() {
    let _serial = serialised();
    forget();
    crate::pending::settle();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow("llm", Rc::clone(&asked), Duration::from_millis(10)));

    // Ask about one line, let the debounce pass so the request actually goes out.
    assert_eq!(ask(&ctx("git ch")), None);
    std::thread::sleep(Duration::from_millis(15));
    assert_eq!(ask(&ctx("git ch")), None);
    assert!(crate::pending::outstanding(), "one request is out");

    // Now the line changes and a second request supersedes the first, twice over. Two asks per
    // line: the first restarts the debounce for the new line, the second sends once it elapses.
    for line in ["git che", "git chec"] {
        assert_eq!(ask(&ctx(line)), None, "the debounce restarts for {line}");
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(ask(&ctx(line)), None, "and then it is sent");
    }
    assert_eq!(asked.borrow().len(), 3, "three requests went out");

    // The last one answers. That balances *one*; the two it superseded must already be balanced.
    answered("llm", "git chec", Some("git checkout".to_string()));
    assert!(
        !crate::pending::outstanding(),
        "nothing is left outstanding once the live request has answered"
    );
    forget();
    crate::pending::settle();
}
