//! What the preview says, for each kind of coordinate.

use super::*;

fn with_prompts(lines: &[&str], line: &str) -> Option<String> {
    oslo_base::prompts::forget();
    // `remember` puts the newest first, so these go in oldest-first to read like a session.
    for typed in lines {
        oslo_base::prompts::remember(typed);
    }
    let out = preview(line);
    oslo_base::prompts::forget();
    out
}

/// **The session axis resolves**, which is the whole feature: the word you are about to reuse,
/// before you commit to it.
#[test]
fn a_session_coordinate_shows_its_value() {
    let session = ["echo first", "cat one.txt two.txt"];
    assert_eq!(
        with_prompts(&session, "wc -l {-1:0:1}").as_deref(),
        Some("one.txt")
    );
    assert_eq!(
        with_prompts(&session, "wc -l {-1:0:-1}").as_deref(),
        Some("two.txt")
    );
    // Two prompts back.
    assert_eq!(
        with_prompts(&session, "echo {-2:0:1}").as_deref(),
        Some("first")
    );
    // The whole line, when no word is asked for.
    assert_eq!(
        with_prompts(&session, "echo {-1:0:}").as_deref(),
        Some("cat one.txt two.txt")
    );
}

/// The command axis says the same thing in the spelling that means it.
#[test]
fn a_command_coordinate_shows_the_command() {
    let session = ["cat one.txt two.txt"];
    assert_eq!(with_prompts(&session, "x {%-1:0}").as_deref(), Some("cat"));
    assert_eq!(
        with_prompts(&session, "x {%-1:1}").as_deref(),
        Some("one.txt")
    );
    assert_eq!(
        with_prompts(&session, "x {%-1}").as_deref(),
        Some("cat one.txt two.txt")
    );
}

/// **A pipeline coordinate cannot be resolved and says so**, rather than showing nothing — which
/// would read as "not recognised", the one thing this exists to disprove.
#[test]
fn a_pipeline_coordinate_says_it_waits() {
    for line in [
        "cat f | ssh {0:0}",
        "cat f | echo {1:0:0}",
        "cat f | echo {%0:0}",
    ] {
        assert_eq!(
            with_prompts(&["earlier"], line).as_deref(),
            Some("at run time"),
            "for {line:?}"
        );
    }
}

/// Nothing to preview is `None` — no annotation drawn at all.
#[test]
fn a_line_with_no_coordinate_previews_nothing() {
    for line in ["echo hello", "mkdir {a,b}", "echo {1..3}", ""] {
        assert_eq!(with_prompts(&["earlier"], line), None, "for {line:?}");
    }
}

/// Out of range reads empty everywhere else in this feature, and says so here.
#[test]
fn reaching_past_the_ring_says_nothing_is_there() {
    assert_eq!(
        with_prompts(&["only one"], "echo {-9:0:0}").as_deref(),
        Some("nothing there")
    );
    assert_eq!(
        with_prompts(&["only one"], "echo {-1:9:0}").as_deref(),
        Some("nothing there")
    );
}

/// Many values are cut, and the count says how many were not shown.
#[test]
fn many_values_are_bounded() {
    let session = ["run a b c d e f g"];
    let shown = with_prompts(&session, "x {%-1:*}").expect("a preview");
    assert!(shown.starts_with("run a b"), "{shown}");
    assert!(shown.contains("(8 values)"), "{shown}");
}

/// The last coordinate in the line is the one being typed, so it is the one previewed.
#[test]
fn the_last_coordinate_is_the_one_shown() {
    let session = ["cat one.txt two.txt"];
    assert_eq!(
        with_prompts(&session, "cp {-1:0:1} {-1:0:-1}").as_deref(),
        Some("two.txt")
    );
}
