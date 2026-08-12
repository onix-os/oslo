use super::*;

fn ctx(line: &str) -> Ctx {
    Ctx {
        line: line.to_string(),
        cursor: line.len(),
        cwd: "/w".to_string(),
        language: "sh".to_string(),
    }
}

fn saying(name: &str, whole: Option<&'static str>) -> Provider {
    Provider {
        name: name.to_string(),
        ask: Ask::Now(Rc::new(move |_| whole.map(str::to_string))),
        only: Only::default(),
    }
}

#[test]
fn nothing_registered_answers_nothing_and_asks_nothing() {
    forget();
    assert!(!any(), "the atomic is what the keystroke path reads");
    assert_eq!(ask(&ctx("git ")), None);
}

#[test]
fn a_provider_answers_the_remainder_not_the_whole_line() {
    forget();
    register(saying("tldr", Some("git commit --amend")));
    assert_eq!(ask(&ctx("git com")), Some("mit --amend".to_string()));
    forget();
}

/// **The invariant.** The ghost is drawn after what you typed and Right accepts it, so an answer
/// that does not continue the line would make that key insert something never suggested.
#[test]
fn an_answer_that_does_not_continue_the_line_is_refused() {
    forget();
    register(saying("wrong", Some("sudo apt update")));
    assert_eq!(ask(&ctx("git com")), None, "not drawn, not trimmed");
    forget();
}

/// Equal is not a continuation: there is nothing to draw, and an empty ghost would light up the
/// accept keys for a suggestion that adds nothing.
#[test]
fn an_answer_equal_to_the_line_is_not_a_suggestion() {
    forget();
    register(saying("echo", Some("git status")));
    assert_eq!(ask(&ctx("git status")), None);
    forget();
}

#[test]
fn the_first_provider_with_an_answer_wins_and_the_rest_are_not_asked() {
    forget();
    let asked = Rc::new(std::cell::Cell::new(0));
    register(saying("first", Some("git status")));
    let counter = Rc::clone(&asked);
    register(Provider {
        name: "second".to_string(),
        only: Only::default(),
        ask: Ask::Now(Rc::new(move |_| {
            counter.set(counter.get() + 1);
            Some("git stash".to_string())
        })),
    });

    assert_eq!(ask(&ctx("git st")), Some("atus".to_string()));
    assert_eq!(asked.get(), 0, "the second was never asked");
    forget();
}

/// One that declines is skipped rather than ending the walk.
#[test]
fn a_provider_that_declines_hands_on_to_the_next() {
    forget();
    register(saying("quiet", None));
    register(saying("loud", Some("git status")));
    assert_eq!(ask(&ctx("git st")), Some("atus".to_string()));
    forget();
}

#[test]
fn registering_the_same_name_twice_replaces_rather_than_doubles() {
    forget();
    register(saying("tldr", Some("git status")));
    register(saying("tldr", Some("git stash")));
    assert_eq!(names(), vec!["tldr".to_string()]);
    assert_eq!(ask(&ctx("git st")), Some("ash".to_string()));
    forget();
}

/// **A slow provider is switched off rather than paid for on every key.** From the outside a shell
/// that stutters looks like oslo being slow, not like somebody's plugin being slow.
#[test]
fn a_provider_that_overruns_its_budget_is_disabled() {
    forget();
    register(Provider {
        name: "slow".to_string(),
        only: Only::default(),
        ask: Ask::Now(Rc::new(|_| {
            std::thread::sleep(BUDGET + Duration::from_millis(5));
            Some("git status".to_string())
        })),
    });

    // Forgiven a few times, because one slow answer is not a slow provider.
    for _ in 0..=FORGIVEN {
        assert_eq!(ask(&ctx("git st")), Some("atus".to_string()));
    }
    assert_eq!(ask(&ctx("git st")), None, "switched off after the grace");
    forget();
}

/// Being disabled must not stop the ones behind it answering.
#[test]
fn a_disabled_provider_does_not_take_the_others_with_it() {
    forget();
    register(Provider {
        name: "slow".to_string(),
        only: Only::default(),
        ask: Ask::Now(Rc::new(|_| {
            std::thread::sleep(BUDGET + Duration::from_millis(5));
            None
        })),
    });
    register(saying("quick", Some("git status")));

    for _ in 0..=FORGIVEN + 1 {
        assert_eq!(ask(&ctx("git st")), Some("atus".to_string()));
    }
    forget();
}

#[test]
fn what_a_provider_is_told_is_the_line_and_where_it_is() {
    forget();
    let seen = Rc::new(RefCell::new(None));
    let into = Rc::clone(&seen);
    register(Provider {
        name: "spy".to_string(),
        only: Only::default(),
        ask: Ask::Now(Rc::new(move |ctx| {
            *into.borrow_mut() = Some(ctx.clone());
            None
        })),
    });
    ask(&ctx("git st"));

    let seen = seen.borrow();
    let seen = seen.as_ref().expect("asked");
    assert_eq!(seen.line, "git st");
    assert_eq!(seen.cursor, 6);
    assert_eq!(seen.language, "sh");
    forget();
}

// ---------------------------------------------------------------- answering later

/// A provider that records what it was asked and answers only when told to.
fn slow(name: &str, asked: Rc<RefCell<Vec<String>>>, debounce: Duration) -> Provider {
    slow_with(name, asked, debounce, Late::Replace, Duration::from_secs(5))
}

fn slow_with(
    name: &str,
    asked: Rc<RefCell<Vec<String>>>,
    debounce: Duration,
    on_late: Late,
    settle: Duration,
) -> Provider {
    Provider {
        name: name.to_string(),
        only: Only::default(),
        ask: Ask::Later {
            request: Rc::new(move |ctx| asked.borrow_mut().push(ctx.line.clone())),
            debounce,
            timeout: Duration::from_secs(5),
            on_late,
            settle,
        },
    }
}

/// **Nothing is asked on the first keystroke.** The debounce is the whole point: ten keys in one
/// word is one question, not ten.
#[test]
fn a_request_waits_for_typing_to_go_quiet() {
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow("llm", Rc::clone(&asked), Duration::from_millis(30)));

    assert_eq!(ask(&ctx("git s")), None, "nothing to draw yet");
    assert!(asked.borrow().is_empty(), "and nothing asked yet");

    std::thread::sleep(Duration::from_millis(35));
    assert_eq!(ask(&ctx("git s")), None);
    assert_eq!(
        asked.borrow().clone(),
        vec!["git s".to_string()],
        "now asked"
    );
    forget();
}

/// Typing on restarts the wait, so what is finally asked is the line you stopped at.
#[test]
fn a_line_that_keeps_changing_is_never_asked_about() {
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow("llm", Rc::clone(&asked), Duration::from_millis(30)));

    for line in ["g", "gi", "git", "git ", "git s"] {
        ask(&ctx(line));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        asked.borrow().is_empty(),
        "still typing: {:?}",
        asked.borrow()
    );

    std::thread::sleep(Duration::from_millis(35));
    ask(&ctx("git s"));
    assert_eq!(asked.borrow().clone(), vec!["git s".to_string()]);
    forget();
}

#[test]
fn an_answer_is_drawn_once_it_arrives() {
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow("llm", Rc::clone(&asked), Duration::ZERO));

    ask(&ctx("git s"));
    assert_eq!(
        asked.borrow().len(),
        1,
        "asked straight away with no debounce"
    );

    answered("llm", "git s", Some("git status".to_string()));
    assert_eq!(ask(&ctx("git s")), Some("tatus".to_string()));
    forget();
}

/// **The bug this design exists to prevent.** An answer to `gi` must never appear under `git `.
#[test]
fn an_answer_to_an_older_line_is_never_drawn() {
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow("llm", Rc::clone(&asked), Duration::ZERO));

    ask(&ctx("gi"));
    // The line moves on, and only then does the answer to the old one arrive.
    ask(&ctx("git "));
    answered("llm", "gi", Some("git status".to_string()));

    assert_eq!(
        ask(&ctx("git ")),
        None,
        "the answer was to a different question"
    );
    forget();
}

/// A reply to something never asked, or a second reply, must not unbalance the editor's wait.
#[test]
fn a_reply_nobody_asked_for_is_ignored() {
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow("llm", Rc::clone(&asked), Duration::ZERO));

    answered("llm", "never asked", Some("git status".to_string()));
    assert_eq!(ask(&ctx("never asked")), None, "not adopted as an answer");

    ask(&ctx("git s"));
    answered("llm", "git s", Some("git status".to_string()));
    answered("llm", "git s", Some("git stash".to_string()));
    assert_eq!(
        ask(&ctx("git s")),
        Some("tatus".to_string()),
        "the second reply is not the answer"
    );
    forget();
}

/// A provider that declines answers nothing rather than leaving the line waiting for ever.
#[test]
fn a_reply_of_nothing_is_a_decline() {
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow("llm", Rc::clone(&asked), Duration::ZERO));

    ask(&ctx("git s"));
    answered("llm", "git s", None);
    assert_eq!(ask(&ctx("git s")), None);
    // And it is not asked again for the same line: it has answered.
    assert_eq!(asked.borrow().len(), 1);
    forget();
}

/// One request per line, however many frames are drawn while it is out.
#[test]
fn a_line_is_asked_about_once_while_the_answer_is_coming() {
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    register(slow("llm", Rc::clone(&asked), Duration::ZERO));

    for _ in 0..5 {
        ask(&ctx("git s"));
    }
    assert_eq!(asked.borrow().len(), 1);
    forget();
}

/// An answer that never comes is given up on, and giving up is not a retry.
///
/// **Asserted on behaviour, not on `pending`'s counter.** That counter is process-wide and every
/// other test here moves it, so reading it would be a race dressed up as an assertion; that it
/// balances is `pending`'s own test.
#[test]
fn a_request_that_is_never_answered_times_out() {
    forget();
    let asked = Rc::new(RefCell::new(Vec::new()));
    let counted = Rc::clone(&asked);
    register(Provider {
        name: "gone".to_string(),
        only: Only::default(),
        ask: Ask::Later {
            request: Rc::new(move |ctx| counted.borrow_mut().push(ctx.line.clone())),
            debounce: Duration::ZERO,
            timeout: Duration::from_millis(20),
            on_late: Late::Replace,
            settle: Duration::from_secs(5),
        },
    });

    ask(&ctx("git s"));
    assert_eq!(asked.borrow().len(), 1);

    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(
        ask(&ctx("git s")),
        None,
        "nothing came, so nothing is drawn"
    );
    assert_eq!(
        asked.borrow().len(),
        1,
        "and not asked again for the same line: a timeout is a decline, not a retry loop"
    );

    // A different line is a different question, and is asked.
    ask(&ctx("git st"));
    assert_eq!(asked.borrow().len(), 2);
    forget();
}

// ---------------------------------------------------------------- what a late answer may do

/// **`fill` never changes what is drawn.** It answers only in the second pass, which runs when every
/// source declined — so a provider set this way can add a suggestion but never take one over.
#[test]
fn a_gap_filler_is_silent_in_its_own_turn() {
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
