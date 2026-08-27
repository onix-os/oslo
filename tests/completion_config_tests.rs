//! What a config's own completions carry, and how they are written back.
//!
//! `oslo.completion.for_command.git` means *I own git*: oslo's own candidates for that command are
//! dropped and the hook's are used instead. That makes this path the only source of candidates for
//! the command it answers for — so anything the merge site does wrong to them is not diluted by
//! other builders, it is the whole menu.
//!
//! Both bugs pinned here lived at that seam rather than in the hook: one in what the candidates
//! *carry*, the other in what is done with them on the way into the line.

mod common;

use oslo::env::Environment;
use oslo::ui::OsloHelper;
use std::sync::{Arc, Mutex};

/// A helper over a shell that knows nothing, with the menu off so nothing tries to draw.
fn helper() -> OsloHelper {
    let mut helper = OsloHelper::new(Arc::new(Mutex::new(Environment::new())));
    helper.set_menu(false);
    helper
}

/// A hook answering a fixed list, whatever it is asked.
fn offering(answers: &'static [&'static str]) -> oslo::ui::completion::CommandCompleter {
    std::rc::Rc::new(move |_command: &str, _prior: &[&str], _current: &str| {
        Some(answers.iter().map(|a| (a.to_string(), None)).collect())
    })
}

fn offered(line: &str) -> Vec<oslo::ui::dropdown::CompletionCandidate> {
    helper().candidates(line, line.len()).1
}

/// **A candidate accepted inside an open quote keeps the quote.**
///
/// The word's `start` points at the *opening* quote, and accepting a candidate overwrites
/// everything from there — so the replacement has to re-supply it. The hook's candidates were
/// inserted raw, so `git commit -m "fi<Tab>` taking `fix: broken pipe` left
/// `git commit -m fix: broken pipe`: the quote gone, and two stray operands where one argument was.
/// Every other builder already ran its value through `quote_replacement`; this one did not.
#[test]
fn a_config_candidate_is_quoted_for_the_context_it_lands_in() {
    oslo::ui::completion::set_command_completer(Some(offering(&["fix: broken pipe"])));

    let inside_quotes = offered(r#"git commit -m "fi"#);
    let first = inside_quotes.first().expect("the hook answered");
    assert_eq!(
        first.replacement, r#""fix: broken pipe""#,
        "the opening quote is re-supplied"
    );
    // The label the user reads is still the plain text.
    assert_eq!(first.display, "fix: broken pipe");

    // Unquoted, the spaces are escaped instead — the same rule every other builder follows.
    let bare = offered("git commit -m fi");
    assert_eq!(
        bare.first().expect("answered").replacement,
        r"fix:\ broken\ pipe"
    );

    oslo::ui::completion::set_command_completer(None);
}

/// **A config's candidates survive `oslo.completion.sources`.**
///
/// They carried no kind at all, and the filter keeps only candidates whose kind is one the config
/// named — so `None` failed that test unconditionally and setting `sh_sources` deleted every one of
/// them. Because this branch *replaces* oslo's own candidates, the menu came back empty and the
/// hook looked broken rather than the filter. `completion/provider.rs` names this hole in its own
/// module doc and closes it only for the provider path.
#[test]
fn a_config_candidate_carries_a_kind_the_filter_can_name() {
    oslo::ui::completion::set_command_completer(Some(offering(&["main", "develop"])));

    let candidates = offered("git checkout ");
    assert!(!candidates.is_empty(), "the hook answered");
    for candidate in &candidates {
        assert_eq!(
            candidate.kind.as_deref(),
            Some("config"),
            "every one is filterable: {candidate:?}"
        );
    }

    oslo::ui::completion::set_command_completer(None);
}

/// **`ctx.arg` counts the words already typed.**
///
/// The field is documented as "1 for the first word after the command", and `words` includes the
/// command — so `git <Tab>` is argument 1 and `git commit <Tab>` is argument 2. It was computed as
/// `len - 1` and then clamped to at least 1, which made *both* of them 1: the documented
/// `if ctx.arg == 2 then return branches() end` never fired, and an `== 1` arm fired twice.
#[test]
fn ctx_arg_counts_the_words_already_typed() {
    use oslo::ui::completion::provider;
    use std::cell::RefCell;

    // Each answer records the `arg` it was told, and offers something so the menu is non-empty.
    thread_local! {
        static SEEN: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }
    provider::forget();
    provider::register(provider::Provider {
        name: "recorder".to_string(),
        kind: "recorder".to_string(),
        when: Some("gitish".to_string()),
        score_offset: 0.0,
        max_items: provider::DEFAULT_MAX_ITEMS,
        min_chars: 0,
        enabled: None,
        answer: std::rc::Rc::new(|ctx: &provider::Ctx| {
            SEEN.with(|seen| seen.borrow_mut().push(ctx.arg));
            vec![provider::Offer {
                display: "anything".to_string(),
                description: None,
            }]
        }),
    });

    let _ = offered("gitish ");
    let _ = offered("gitish commit ");
    let _ = offered("gitish commit --amend ");

    let seen = SEEN.with(|seen| seen.borrow().clone());
    assert_eq!(
        seen,
        vec![1, 2, 3],
        "the first argument is 1 and each further word counts"
    );
    provider::forget();
}
