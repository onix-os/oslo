use super::*;

fn ctx(command: &str, current: &str) -> Ctx {
    Ctx {
        command: command.to_string(),
        words: vec![command.to_string()],
        current: current.to_string(),
        arg: 1,
        cwd: "/w".to_string(),
    }
}

fn offering(
    name: &str,
    when: Option<&str>,
    offers: &'static [(&'static str, &'static str)],
) -> Provider {
    Provider {
        name: name.to_string(),
        kind: "example".to_string(),
        when: when.map(str::to_string),
        score_offset: 0.0,
        max_items: DEFAULT_MAX_ITEMS,
        min_chars: 0,
        enabled: None,
        answer: Rc::new(move |_| {
            offers
                .iter()
                .map(|(display, description)| Offer {
                    display: display.to_string(),
                    description: Some(description.to_string()),
                })
                .collect()
        }),
    }
}

#[test]
fn nothing_registered_offers_nothing() {
    forget();
    assert!(!any());
    assert!(offers(&ctx("git", "")).is_empty());
}

#[test]
fn what_a_provider_offers_carries_its_kind_and_description() {
    forget();
    register(offering(
        "tldr",
        None,
        &[("git commit --amend", "change the last commit")],
    ));
    let out = offers(&ctx("git", ""));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0.display, "git commit --amend");
    assert_eq!(
        out[0].0.description.as_deref(),
        Some("change the last commit")
    );
    // **A kind, unlike `for_command`.** Without one, `oslo.completion.sources` filters it out.
    assert_eq!(out[0].0.kind.as_deref(), Some("example"));
    forget();
}

/// A provider that named a command answers for that one and stays quiet elsewhere.
#[test]
fn a_provider_can_answer_for_one_command_or_for_any() {
    forget();
    register(offering("gitonly", Some("git"), &[("git status", "")]));
    register(offering("always", None, &[("anything", "")]));

    let for_git: Vec<String> = offers(&ctx("git", ""))
        .into_iter()
        .map(|(c, _)| c.display)
        .collect();
    assert!(for_git.contains(&"git status".to_string()));
    assert!(for_git.contains(&"anything".to_string()));

    let for_ls: Vec<String> = offers(&ctx("ls", ""))
        .into_iter()
        .map(|(c, _)| c.display)
        .collect();
    assert_eq!(for_ls, vec!["anything".to_string()], "git's stayed home");
    forget();
}

/// **One provider cannot flood the menu.** A database dumped whole would push everything else off
/// the screen.
#[test]
fn max_items_bounds_what_one_provider_contributes() {
    forget();
    register(Provider {
        name: "many".to_string(),
        kind: "example".to_string(),
        when: None,
        score_offset: 0.0,
        max_items: 3,
        min_chars: 0,
        enabled: None,
        answer: Rc::new(|_| {
            (0..100)
                .map(|n| Offer {
                    display: format!("item{n}"),
                    description: None,
                })
                .collect()
        }),
    });
    assert_eq!(offers(&ctx("git", "")).len(), 3);
    forget();
}

#[test]
fn the_score_offset_comes_back_with_each_offer() {
    forget();
    let mut provider = offering("tldr", None, &[("git status", "")]);
    provider.score_offset = 20.0;
    register(provider);
    assert_eq!(offers(&ctx("git", ""))[0].1, 20.0);
    forget();
}

#[test]
fn an_offer_with_no_text_is_not_an_offer() {
    forget();
    register(offering("empty", None, &[("", "nothing to insert")]));
    assert!(offers(&ctx("git", "")).is_empty());
    forget();
}

#[test]
fn registering_the_same_name_twice_replaces_rather_than_doubles() {
    forget();
    register(offering("tldr", None, &[("first", "")]));
    register(offering("tldr", None, &[("second", "")]));
    assert_eq!(names(), vec!["tldr".to_string()]);
    assert_eq!(offers(&ctx("git", ""))[0].0.display, "second");
    forget();
}
