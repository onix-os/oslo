//! The same assertions `history`'s help is held to, because the two must read alike.

use super::*;

#[test]
fn the_overview_describes_every_subcommand() {
    let plain = MENU.overview(Paint::plain());
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
        let help = MENU
            .subcommand(sub.name, Paint::plain())
            .expect("every listed name answers");
        assert!(help.contains(sub.name), "{}", sub.name);
        assert!(help.contains(sub.about), "{}", sub.name);
        for (flag, about) in sub.flags {
            assert!(help.contains(flag), "{}: {flag} missing", sub.name);
            assert!(help.contains(about), "{}: {flag} undescribed", sub.name);
        }
    }
}

/// The things somebody would otherwise find out the hard way.
#[test]
fn the_surprises_are_written_down() {
    let checks = [
        ("list", "no install verb"),
        ("doctor", "the plugin is loaded"),
        ("test", "a home with nothing in it"),
    ];
    for (name, phrase) in checks {
        let help = MENU.subcommand(name, Paint::plain()).expect("listed");
        assert!(help.contains(phrase), "{name} does not mention {phrase:?}");
    }
}

#[test]
fn a_name_nobody_listed_has_no_help() {
    assert!(MENU.subcommand("nonesuch", Paint::plain()).is_none());
}

/// The overview and the tool table must agree about what exists.
#[test]
fn the_headings_match_the_house_style() {
    let plain = MENU.overview(Paint::plain());
    assert!(plain.starts_with("USAGE\n"), "{plain}");
    assert!(plain.contains("\nSUBCOMMANDS\n"), "{plain}");
    let doctor = MENU.subcommand("doctor", Paint::plain()).expect("listed");
    assert!(doctor.starts_with("USAGE\n"), "{doctor}");
    // ARGUMENTS is the block that lists flags, and none of these three takes one any more --
    // `--yes` went with `install`. Asserted rather than dropped, so growing a flag without a
    // heading is still caught.
    assert!(SUBCOMMANDS.iter().all(|sub| sub.flags.is_empty()));
    assert!(!doctor.contains("\nARGUMENTS\n"), "{doctor}");
}
