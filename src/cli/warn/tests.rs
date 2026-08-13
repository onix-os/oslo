use super::*;

/// Nothing wrong means nothing printed. A box that appears every run teaches people to skip the
/// top of the output, which costs exactly the one time it matters.
#[test]
fn a_healthy_install_says_nothing() {
    let facts = Facts::default();
    assert!(facts.findings().is_empty());
    assert_eq!(box_of(&facts, Paint::plain(), 0), "");
}

/// Each problem is one line, and the line names the path and what is actually there — "not oslo"
/// alone would leave you running `ls -l` to find out what you have instead.
#[test]
fn each_problem_is_one_line_that_names_it() {
    let facts = Facts {
        foreign_sh: vec![("/bin/sh".into(), "dash".into())],
        bad_mode: Some(0o644),
    };
    let found = facts.findings();
    assert_eq!(found.len(), 2);
    // Worst first: the one that fails for somebody, then the one that is merely true.
    assert_eq!(found[0].0, Level::Warning);
    assert!(found[0].1.contains("0644"), "{}", found[0].1);
    assert!(
        found[0].1.contains("0755"),
        "the fix is named: {}",
        found[0].1
    );
    assert_eq!(
        found[1],
        (Level::Hint, "/bin/sh is dash, not oslo".to_string())
    );
}

/// **`/bin/sh` is a hint, not a warning.** Nothing is broken by it — oslo runs, and so do the
/// scripts, they just do not run *in* oslo. Red is what makes the eye stop, and spending it here is
/// how a warning stops being read for the day something really is wrong.
#[test]
fn a_foreign_sh_is_only_a_hint() {
    let facts = Facts {
        foreign_sh: vec![("/bin/sh".into(), "dash".into())],
        ..Facts::default()
    };
    let drawn = box_of(&facts, Paint::plain(), 0);
    assert!(drawn.contains("hint"), "{drawn}");
    assert!(!drawn.contains("warning"), "{drawn}");

    // Dim, not red — the colour says the same thing the word does.
    let painted = box_of(&facts, Paint::at(oslo::ui::theme::Depth::True), 0);
    assert!(
        !painted.contains("224;64;64"),
        "a hint in warning red: {painted:?}"
    );
}

/// The box is a box: every row the same width, and the content inside it. Two kinds of finding are
/// two boxes, because one box titled for the more serious would be lying about the other.
#[test]
fn the_box_lines_up() {
    let facts = Facts {
        foreign_sh: vec![("/bin/sh".into(), "dash".into())],
        bad_mode: Some(0o600),
    };
    let drawn = box_of(&facts, Paint::plain(), 0);
    let rows: Vec<&str> = drawn.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(rows.len(), 6, "two boxes of three rows: {drawn}");

    for box_rows in rows.chunks(3) {
        let width = box_rows[0].chars().count();
        for row in box_rows {
            assert_eq!(row.chars().count(), width, "ragged row {row:?} in {drawn}");
        }
    }
    assert!(rows[0].contains("warning"), "the worse one first: {drawn}");
    assert!(rows[3].contains("hint"), "{drawn}");
    assert!(drawn.contains("/bin/sh is dash"));
}

/// A line longer than the title widens the box rather than spilling out of it.
#[test]
fn a_long_line_widens_the_box() {
    let facts = Facts {
        foreign_sh: vec![("/usr/bin/sh".into(), "a-very-long-shell-name-indeed".into())],
        ..Facts::default()
    };
    let drawn = box_of(&facts, Paint::plain(), 0);
    let rows: Vec<&str> = drawn.lines().filter(|l| !l.is_empty()).collect();
    let width = rows[0].chars().count();
    assert!(rows.iter().all(|r| r.chars().count() == width), "{drawn}");
    assert!(width > 40, "the box grew to fit: {width}");
}

/// **Plain when there is no colour, red when there is.** A warning has to survive `NO_COLOR` and a
/// pipe, and it is the one part of the help where losing the colour loses meaning — so the box
/// itself carries it, not just the paint.
#[test]
fn the_box_is_red_only_when_colour_is_wanted() {
    let facts = Facts {
        bad_mode: Some(0o600),
        ..Facts::default()
    };
    assert!(!box_of(&facts, Paint::plain(), 0).contains('\x1b'));

    let painted = box_of(&facts, Paint::at(oslo::ui::theme::Depth::True), 0);
    assert!(painted.contains("38;2;224;64;64"), "{painted:?}");
}

/// **The box matches the page it sits under.** Narrower than the text above it, it reads as a
/// stray fragment rather than as the page's own footer.
#[test]
fn the_box_widens_to_the_page() {
    let facts = Facts {
        bad_mode: Some(0o600),
        ..Facts::default()
    };
    let drawn = box_of(&facts, Paint::plain(), 78);
    for row in drawn.lines().filter(|l| !l.is_empty()) {
        assert_eq!(row.chars().count(), 78, "{row:?} in {drawn}");
    }
}

/// But it grows past the page rather than truncating: a warning that has been cut off is worse
/// than a ragged edge.
#[test]
fn a_line_too_long_for_the_page_still_fits_whole() {
    let long = "a-shell-with-a-really-quite-unreasonably-long-name-here";
    let facts = Facts {
        foreign_sh: vec![("/bin/sh".into(), long.into())],
        ..Facts::default()
    };
    let drawn = box_of(&facts, Paint::plain(), 20);
    assert!(drawn.contains(long), "the text was cut: {drawn}");
    let width = drawn.lines().next().expect("a top edge").chars().count();
    assert!(width > 20, "the box grew past the page: {width}");
}

/// **Two names for one file is one problem.** `/bin` is a symlink to `usr/bin` on every
/// distribution that did the usrmerge, so `/bin/sh` and `/usr/bin/sh` are the same inode and
/// reporting both said the same thing twice.
///
/// Asserted against the real filesystem, because that is where the duplication came from: on a
/// merged-`/usr` machine `detect()` must find at most one foreign `sh`, and on an unmerged one two
/// genuinely different files would deserve two lines.
#[test]
fn one_file_under_two_names_is_reported_once() {
    let merged = std::fs::canonicalize("/bin/sh").ok() == std::fs::canonicalize("/usr/bin/sh").ok();
    let found = detect().foreign_sh.len();
    if merged {
        assert!(
            found <= 1,
            "one file reported {found} times: {:?}",
            detect().foreign_sh
        );
    }
}

/// A missing path is not a misconfiguration — plenty of systems have no `/usr/bin/sh`, and
/// warning about one would be a warning nobody can act on.
#[test]
fn a_path_that_does_not_exist_is_not_a_problem() {
    assert_eq!(foreign(Path::new("/no/such/sh/anywhere"), None), None);
}

/// A path that resolves to *us* is the whole point of the check passing.
#[test]
fn a_path_that_is_us_is_not_foreign() {
    let me = std::fs::canonicalize("/proc/self/exe").expect("our own binary");
    assert_eq!(foreign(&me, Some(&me)), None);
    // And the same path judged against a different binary is foreign.
    assert!(foreign(&me, Some(Path::new("/somewhere/else"))).is_some());
}

/// Executable by everyone is the test, not an exact `0755` — a distribution shipping `0555` is
/// right, and complaining would be noise nobody should act on.
#[test]
fn only_a_mode_that_blocks_execution_is_reported() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("binary");
    std::fs::write(&path, b"#!/bin/sh\n").expect("write");

    let set = |mode: u32| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    };

    for fine in [0o755, 0o555, 0o777, 0o711] {
        set(fine);
        assert_eq!(bad_mode(&path), None, "{fine:04o} runs for everybody");
    }
    for broken in [0o644, 0o700, 0o750, 0o600] {
        set(broken);
        assert_eq!(
            bad_mode(&path),
            Some(broken),
            "{broken:04o} blocks somebody"
        );
    }
}
