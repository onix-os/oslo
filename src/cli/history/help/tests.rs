use super::*;

#[test]
fn the_overview_describes_every_subcommand() {
    let plain = text(Paint::plain());
    assert!(!plain.contains('\x1b'), "plain is plain");
    for sub in SUBCOMMANDS {
        assert!(plain.contains(sub.name), "{} is missing", sub.name);
        assert!(
            plain.contains(sub.about),
            "{} has no description in the overview",
            sub.name
        );
    }
    assert!(plain.contains("--help"), "it says where to go for more");
}

#[test]
fn each_subcommand_documents_its_own_arguments() {
    for sub in SUBCOMMANDS {
        let help = subcommand(sub.name, Paint::plain()).expect("every listed name answers");
        assert!(help.contains(sub.name), "{}", sub.name);
        assert!(help.contains(sub.about), "{}", sub.name);
        for (flag, about) in sub.flags {
            assert!(help.contains(flag), "{}: {flag} missing", sub.name);
            assert!(help.contains(about), "{}: {flag} undescribed", sub.name);
        }
        if sub.name != "search" {
            assert!(!help.contains("--contains"), "{} leaked search", sub.name);
        }
    }
}

#[test]
fn the_surprises_are_written_down() {
    let checks = [
        ("verify", "never creates"),
        ("sync", "order picks no winner"),
        ("prune", "Local only"),
        ("delete", "tombstone"),
        ("import", "twice"),
        ("backup", "consistent snapshot"),
    ];
    for (name, expected) in checks {
        let help = subcommand(name, Paint::plain()).expect("a known subcommand");
        let said = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            said.to_lowercase().contains(&expected.to_lowercase()),
            "{name} does not mention {expected:?}:\n{help}"
        );
    }
}

#[test]
fn an_unknown_subcommand_has_no_help() {
    assert!(subcommand("nope", Paint::plain()).is_none());
    assert!(subcommand("", Paint::plain()).is_none());
}

#[test]
fn colour_follows_the_shared_painter() {
    assert!(text(Paint::at(oslo::ui::theme::Depth::Ansi256)).contains("\x1b["));
    assert!(
        subcommand("sync", Paint::at(oslo::ui::theme::Depth::Ansi256))
            .expect("sync")
            .contains("\x1b[")
    );
    assert!(
        !subcommand("sync", Paint::plain())
            .expect("sync")
            .contains('\x1b')
    );
}
