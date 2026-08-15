//! Every menu in the binary, held to the same assertions.
//!
//! This is the file that makes the uniformity real. A tool that grows a help page of its own shape
//! is not caught by review — it is caught here, or it is not caught.

use super::*;
use crate::cli::help::Paint;

/// Every menu there is. **Add one here when you add one anywhere**, or it is untested.
fn every_menu() -> Vec<(&'static str, &'static Menu)> {
    // Every entry below the first five is behind a feature, so a build without them adds nothing.
    #[allow(unused_mut)]
    let mut all: Vec<(&'static str, &'static Menu)> = vec![
        ("history", &crate::cli::history::help::MENU),
        ("macros", &crate::cli::macros::help::MENU),
        ("config", &crate::cli::config::MENU),
        ("hook", &crate::cli::hook::MENU),
        ("profile", &crate::cli::profile::help::MENU),
        ("profile key", &crate::cli::profile::help::KEY),
    ];
    #[cfg(feature = "direnv")]
    all.push(("direnv", &crate::cli::direnv::MENU));
    #[cfg(feature = "scratch")]
    all.push(("scratch", &crate::cli::scratch::MENU));
    #[cfg(feature = "plugin")]
    all.push(("plugin", &crate::cli::plugin::help::MENU));
    #[cfg(feature = "secrets")]
    {
        all.push(("secret", &crate::cli::secret::help::MENU));
        all.push(("secret key", &crate::cli::secret::help::KEY));
        all.push(("secret recipient", &crate::cli::secret::help::RECIPIENT));
        all.push(("secret cipher", &crate::cli::secret::help::CIPHER));
    }
    all
}

/// The headings, in the order and the spelling every page uses.
#[test]
fn every_page_has_the_house_headings() {
    for (name, menu) in every_menu() {
        let overview = menu.overview(Paint::plain());
        assert!(overview.starts_with("USAGE\n"), "{name}:\n{overview}");
        assert!(
            overview.contains(&format!("\n{}\n", menu.heading)),
            "{name} does not head its list with {}:\n{overview}",
            menu.heading
        );
        assert!(
            overview.contains("  oslo "),
            "{name}'s usage line does not start with oslo:\n{overview}"
        );
    }
}

/// The USAGE line names the path that reaches this menu, which is what a nested one gets wrong.
#[test]
fn every_page_says_how_it_is_reached() {
    for (name, menu) in every_menu() {
        assert_eq!(
            menu.path.join(" "),
            name,
            "{name}'s path does not match where it is registered"
        );
        let called = format!("oslo {name}");
        assert!(
            menu.overview(Paint::plain()).contains(&called),
            "{name} does not say `{called}`"
        );
    }
}

/// Every row is reachable, says something, and is listed above.
#[test]
fn every_row_has_a_page_and_the_page_is_listed() {
    for (name, menu) in every_menu() {
        let overview = menu.overview(Paint::plain());
        for sub in menu.subs {
            assert!(!sub.about.is_empty(), "{name} {}: says nothing", sub.name);
            assert!(
                overview.contains(sub.name),
                "{name} {}: is not in the list",
                sub.name
            );
            let page = menu
                .subcommand(sub.name, Paint::plain())
                .unwrap_or_else(|| panic!("{name} {}: has no page", sub.name));
            assert!(page.starts_with("USAGE\n"), "{name} {}:\n{page}", sub.name);
            assert!(
                page.contains(&format!("oslo {name} {}", sub.name)),
                "{name} {}'s page does not name it:\n{page}",
                sub.name
            );
            // **A row that leads to a menu of its own carries no arguments and no note**, because
            // that menu is what its `--help` answers with and the row's copy would never be read.
            if menu.nested.iter().any(|deeper| deeper.leaf() == sub.name) {
                assert!(
                    sub.flags.is_empty() && sub.note.is_empty(),
                    "{name} {}: says what its own menu says, where nobody will see it",
                    sub.name
                );
                continue;
            }
            for (flag, about) in sub.flags {
                assert!(page.contains(flag), "{name} {}: {flag} missing", sub.name);
                assert!(
                    page.contains(about),
                    "{name} {}: {flag} undescribed",
                    sub.name
                );
            }
        }
    }
}

/// A name nobody listed has no page, in every menu.
#[test]
fn nothing_invents_a_page() {
    for (name, menu) in every_menu() {
        assert!(
            menu.subcommand("nonesuch", Paint::plain()).is_none(),
            "{name} invented one"
        );
        assert!(menu.subcommand("", Paint::plain()).is_none(), "{name}");
    }
}

/// Plain is plain, and colour goes through the one painter.
#[test]
fn colour_is_the_shared_painter_or_nothing() {
    for (name, menu) in every_menu() {
        let plain = menu.overview(Paint::plain());
        assert!(!plain.contains('\x1b'), "{name} paints when told not to");
        let painted = menu.overview(Paint::at(oslo::ui::theme::Depth::Ansi256));
        assert!(painted.contains("\x1b["), "{name} never paints");
    }
}

/// `oslo TOOL SUB --help` and `oslo TOOL --help SUB` are the same question everywhere.
#[test]
fn both_spellings_reach_the_same_page() {
    let said = |words: &[&str]| words.iter().map(|w| (*w).to_string()).collect::<Vec<_>>();
    for (name, menu) in every_menu() {
        let sub = menu.subs[0].name;
        let page = menu.subcommand(sub, Paint::plain());
        assert_eq!(
            menu.asked(&said(&[sub, "--help"]), Paint::plain()),
            page,
            "{name}: `{sub} --help`"
        );
        assert_eq!(
            menu.asked(&said(&["--help", sub]), Paint::plain()),
            page,
            "{name}: `--help {sub}`"
        );
        // A bare --help is the overview, and a name nobody listed falls back to it rather than to
        // nothing at all.
        let overview = Some(menu.overview(Paint::plain()));
        assert_eq!(menu.asked(&said(&["--help"]), Paint::plain()), overview);
        assert_eq!(
            menu.asked(&said(&["--help", "nonesuch"]), Paint::plain()),
            overview,
            "{name}: an unknown name should still get the list"
        );
        // And an ordinary invocation is work, not a page.
        assert!(
            menu.asked(&said(&[sub, "argument"]), Paint::plain())
                .is_none(),
            "{name}: `{sub} argument` was mistaken for a help request"
        );
    }
}

/// The line pointing at the per-row pages appears exactly where those pages exist.
#[test]
fn the_pointer_matches_what_is_behind_it() {
    for (name, menu) in every_menu() {
        let overview = menu.overview(Paint::plain());
        let deeper = menu
            .subs
            .iter()
            .any(|sub| !sub.flags.is_empty() || !sub.note.is_empty());
        assert_eq!(
            overview.contains("--help` for that subcommand's arguments"),
            deeper,
            "{name} points somewhere that is not there, or fails to point:\n{overview}"
        );
    }
}
