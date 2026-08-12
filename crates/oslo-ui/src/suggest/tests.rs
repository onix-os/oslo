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
        answer: Rc::new(move |_| whole.map(str::to_string)),
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
        answer: Rc::new(move |_| {
            counter.set(counter.get() + 1);
            Some("git stash".to_string())
        }),
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
        answer: Rc::new(|_| {
            std::thread::sleep(BUDGET + Duration::from_millis(5));
            Some("git status".to_string())
        }),
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
        answer: Rc::new(|_| {
            std::thread::sleep(BUDGET + Duration::from_millis(5));
            None
        }),
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
        answer: Rc::new(move |ctx| {
            *into.borrow_mut() = Some(ctx.clone());
            None
        }),
    });
    ask(&ctx("git st"));

    let seen = seen.borrow();
    let seen = seen.as_ref().expect("asked");
    assert_eq!(seen.line, "git st");
    assert_eq!(seen.cursor, 6);
    assert_eq!(seen.language, "sh");
    forget();
}
