use super::*;

/// **A plain render has no escapes at all.** This is what `NO_COLOR=1`, a `dumb` terminal and a
/// pipe all reduce to, and an escape leaking through would land in somebody's `grep` or file.
#[test]
fn plain_help_contains_no_escapes() {
    for text in [short(Paint::plain()), details(Paint::plain())] {
        assert!(
            !text.contains('\x1b'),
            "an escape survived a plain render: {text:?}"
        );
    }
}

/// And a coloured render does emit them, or the detection is doing nothing.
#[test]
fn a_colour_render_is_actually_coloured() {
    let painted = short(Paint {
        depth: Depth::Ansi256,
    });
    assert!(painted.contains('\x1b'), "nothing was painted");
}

/// Colour changes only the painting. Stripped back down, the two renders are the same text — so a
/// piped `--help` cannot quietly be a *different* help from the one on screen.
#[test]
fn colour_does_not_change_the_words() {
    let painted = short(Paint {
        depth: Depth::Ansi256,
    });
    assert_eq!(strip(&painted), short(Paint::plain()));
}

/// The description column lines up. Padding is computed on the unpainted width, and getting that
/// wrong is invisible in a plain test and ragged on a real terminal — so it is asserted on the
/// *coloured* render, stripped afterwards.
#[test]
fn the_columns_line_up_even_when_painted() {
    let painted = strip(&short(Paint {
        depth: Depth::Ansi256,
    }));
    let at: Vec<usize> = painted
        .lines()
        .filter(|line| line.starts_with("  -") || line.starts_with("  oslo-"))
        .filter_map(|line| {
            line.find("  ")
                .map(|_| line.len() - line.trim_start().len())
        })
        .collect();
    assert!(!at.is_empty(), "no option rows were found");
    assert!(
        at.iter().all(|&n| n == 2),
        "option rows are indented by two"
    );
}

/// Every tool in the table is listed, so a tool cannot exist and be undiscoverable.
#[test]
fn every_tool_appears_in_the_help() {
    let text = short(Paint::plain());
    for tool in TOOLS {
        assert!(
            text.contains(tool.name),
            "{} is missing from the help",
            tool.name
        );
        assert!(text.contains(tool.about), "{}: no description", tool.name);
    }
}

/// **The documented variable name is the one the code reads.** `$OSLO_PROFILE` is the only one
/// whose spelling is pinned in a constant; asserting against it catches the rename that updates
/// the code and leaves the help describing a variable that no longer does anything.
#[test]
fn the_profile_variable_is_named_as_the_code_spells_it() {
    let text = short(Paint::plain());
    assert!(
        text.contains(oslo::track::profile::ENV),
        "the help must name {}: {text}",
        oslo::track::profile::ENV
    );
}

/// Every listed variable is `OSLO_`-prefixed or a convention oslo did not invent. A one-off name
/// in here would be a setting nobody could guess and nothing else honours.
#[test]
fn the_environment_section_lists_only_real_conventions() {
    const BORROWED: &[&str] = &["NO_COLOR", "XDG_CONFIG_HOME", "XDG_DATA_HOME"];
    for (name, about) in ENVIRONMENT {
        assert!(
            name.starts_with("OSLO_") || BORROWED.contains(name),
            "{name} is neither oslo's nor a standard"
        );
        assert!(!about.is_empty(), "{name}: needs a description");
    }
}

/// The synopsis shows how a tool is run, beside the two shell forms.
#[test]
fn the_help_shows_how_to_run_a_tool() {
    let text = short(Paint::plain());
    assert!(text.contains("oslo <tool>"), "{text}");
}

/// The short help stays short, and points at where the long form lives.
#[test]
fn the_short_help_defers_to_details() {
    let text = short(Paint::plain());
    assert!(text.contains("--help --details"));
    assert!(
        !text.contains("xtrace"),
        "the option reference belongs in --details"
    );
}

/// **`--details` documents every settable option.** The table is the source, so an option added
/// to the shell turns up here without anybody writing it down a second time.
#[test]
fn details_covers_every_shell_option() {
    let text = details(Paint::plain());
    for option in ALL {
        let Some(name) = option.name else { continue };
        assert!(text.contains(name), "{name} is missing from --details");
        assert!(
            text.contains(option.about),
            "{name}: its description is missing"
        );
    }
}

/// An option oslo accepts but does not act on says so. That is the one thing a script cannot
/// detect by running it, so the reference is where it has to be admitted.
#[test]
fn details_admits_what_is_not_implemented() {
    let text = details(Paint::plain());
    assert!(text.contains("not implemented:"));
    // `hashall` is one of them, and its reason is the table's, not a second copy.
    let hashall = ALL
        .iter()
        .find(|o| o.name == Some("hashall"))
        .expect("hashall is an option");
    assert!(text.contains(hashall.unsupported.expect("hashall is refused")));
}

/// `--details` is a superset: everything in the short help is still there.
#[test]
fn details_includes_the_short_help() {
    assert!(details(Paint::plain()).starts_with(&short(Paint::plain())));
}

/// **Width is what a terminal draws, not what a `String` holds.** An escape sequence occupies
/// bytes and no columns, so measuring the painted text would report a page far wider than it is —
/// and the warning box sized from that would run off the edge of the screen.
#[test]
fn width_ignores_the_escapes() {
    let painted = short(Paint {
        depth: Depth::Ansi256,
    });
    assert!(painted.contains('\x1b'), "nothing was painted");
    assert_eq!(
        widest(&painted),
        widest(&short(Paint::plain())),
        "colour changed the measured width"
    );
}

/// A pipe is not a terminal, so a redirected `--help` is never painted.
#[test]
fn a_pipe_is_never_painted() {
    // `cargo test` runs with stdout captured, which is exactly the case being asserted.
    assert_eq!(Paint::detect().depth, Depth::None);
}

/// Remove every escape sequence, so two renders can be compared as text.
fn strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        // CSI: `ESC [ ... final`, where the final byte is 0x40..=0x7e.
        for c in chars.by_ref() {
            if ('\x40'..='\x7e').contains(&c) && c != '[' {
                break;
            }
        }
    }
    out
}
