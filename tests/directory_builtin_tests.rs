//! `cd`, `pwd` and the directory stack, run through the real binary.
//!
//! The point of most of these is the difference between the path a user *walked* and the path
//! the kernel resolved: a shell that only ever reports `getcwd()` answers `cd link; pwd` with
//! the symlink's target, which is not what any other shell — or any script — expects.

mod common;

use common::run_in;
use std::path::PathBuf;

/// A scratch directory with every symlink already resolved, so an expectation built from it can
/// be compared against `pwd` output.
fn scratch() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path().canonicalize().expect("canonicalize tempdir");
    (dir, base)
}

// --- logical versus physical ---

#[test]
fn a_symlinked_directory_stays_in_pwd() {
    let (_dir, base) = scratch();
    let r = run_in(
        &base,
        "mkdir real; ln -s real link; cd link; pwd; echo \"$PWD\"",
    );
    let want = format!("{}/link", base.display());
    assert_eq!(r.out(), format!("{want}\n{want}"), "stderr: {}", r.stderr);
}

#[test]
fn dash_p_resolves_the_symlink() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir real; ln -s real link; cd -P link; pwd");
    assert_eq!(r.out(), format!("{}/real", base.display()));
}

#[test]
fn pwd_p_reports_the_physical_path_after_a_logical_cd() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir real; ln -s real link; cd link; pwd -P");
    assert_eq!(r.out(), format!("{}/real", base.display()));
}

#[test]
fn dotdot_cancels_the_name_not_the_target() {
    let (_dir, base) = scratch();
    // Physically, `link/sub/..` is `real`; logically it is `link`, which is where the user came
    // from and where every other shell puts them back.
    let r = run_in(
        &base,
        "mkdir -p real/sub; ln -s real link; cd link/sub; cd ..; pwd",
    );
    assert_eq!(r.out(), format!("{}/link", base.display()));
}

#[test]
fn a_path_whose_components_do_not_all_exist_is_rejected() {
    let (_dir, base) = scratch();
    // Cancelling `..` first would make this a silent no-op that reports success.
    let r = run_in(&base, "mkdir b; cd nosuch/../b; echo \"$?\"; pwd");
    assert_eq!(r.out(), format!("1\n{}", base.display()));
}

// --- the option matrix ---

#[test]
fn double_dash_ends_the_options() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir -- -P; cd -- -P; pwd");
    assert_eq!(
        r.out(),
        format!("{}/-P", base.display()),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn a_second_operand_is_a_usage_error_that_does_not_move_the_shell() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir a b; cd a b; echo \"$?\"; pwd");
    assert_eq!(r.out(), format!("2\n{}", base.display()));
    assert!(!r.stderr.is_empty(), "the shell must say why it refused");
}

#[test]
fn an_unknown_option_is_refused() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "cd -x; echo \"$?\"");
    assert_eq!(r.out(), "2");
    assert!(!r.stderr.is_empty());
}

#[test]
fn cd_dash_returns_to_oldpwd_and_announces_it() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir a; cd a; cd ..; cd -; pwd");
    let want = format!("{}/a", base.display());
    assert_eq!(r.out(), format!("{want}\n{want}"));
}

// --- what a script gets, which is what it always got ---

/// A script has no store to consult, so there is nothing for `cd` to be clever with: the
/// diagnostic and the status are the ones POSIX asks for and the shell has always given.
#[test]
fn cd_to_a_missing_directory_fails_with_one_and_says_so() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "cd nosuchplace; echo \"$?\"; pwd");
    assert_eq!(r.out(), format!("1\n{}", base.display()));
    assert!(
        r.err().starts_with("oslo: cd: nosuchplace: "),
        "stderr was {:?}",
        r.stderr
    );
}

/// `cd ""` is a usage mistake rather than a missing directory, and keeps its own wording.
#[test]
fn cd_to_the_empty_string_is_a_null_directory() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "cd ''; echo \"$?\"; pwd");
    assert_eq!(r.out(), format!("1\n{}", base.display()));
    assert_eq!(r.err(), "oslo: cd: null directory");
}

/// `root` is a word `cd` knows, but only once the filesystem has said no. A directory of that
/// name is a directory, and it wins — which is what saves anyone who has one.
#[test]
fn a_directory_named_root_is_just_a_directory() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir root; cd root; pwd");
    assert_eq!(
        r.out(),
        format!("{}/root", base.display()),
        "stderr: {}",
        r.stderr
    );
}

/// `prevd` and `nextd` are gone: `cd -` and `cd -N` reach every entry of the ring between them,
/// and walking it with a cursor set `$PWD` behind `$OLDPWD`'s back.
#[test]
fn walking_the_ring_forwards_is_no_longer_a_builtin() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "prevd; echo \"$?\"; nextd; echo \"$?\"");
    assert_eq!(r.out(), "127\n127", "stderr: {}", r.stderr);
}

/// The ring is "where the shell has been", so it is written by the move rather than by `cd`.
/// Recorded from `cd`'s own arm it silently omitted every `pushd`, and `cd -2` then counted back
/// through a history with holes in it.
#[test]
fn every_builtin_that_moves_the_shell_enters_the_ring() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir a b; cd a; pushd ../b >/dev/null; cd ..; dirh");
    assert_eq!(
        r.lines(),
        vec![
            format!("0  {}", base.display()),
            format!("1  {}/b", base.display()),
            format!("2  {}/a", base.display()),
        ],
        "stderr: {}",
        r.stderr
    );
}

// --- CDPATH ---

#[test]
fn cdpath_finds_the_directory_and_echoes_where_it_landed() {
    let (_dir, base) = scratch();
    let r = run_in(
        &base,
        "mkdir -p top/inner; CDPATH=$(pwd)/top; cd inner; pwd",
    );
    let want = format!("{}/top/inner", base.display());
    assert_eq!(r.out(), format!("{want}\n{want}"), "stderr: {}", r.stderr);
}

#[test]
fn an_empty_cdpath_element_is_the_current_directory_and_is_not_echoed() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir -p top; CDPATH=:/nowhere; cd top; pwd");
    assert_eq!(r.out(), format!("{}/top", base.display()));
}

#[test]
fn cdpath_is_not_consulted_for_an_anchored_path() {
    let (_dir, base) = scratch();
    let r = run_in(
        &base,
        "mkdir -p top/inner; CDPATH=$(pwd)/top; cd ./inner; echo \"$?\"",
    );
    assert_eq!(r.out(), "1");
}

// --- the directory stack ---

#[test]
fn pushd_with_no_operand_exchanges_the_top_two_entries() {
    let (_dir, base) = scratch();
    let r = run_in(
        &base,
        "mkdir a b; pushd a >/dev/null; pushd ../b >/dev/null; pushd; pwd",
    );
    let a = format!("{}/a", base.display());
    let b = format!("{}/b", base.display());
    assert_eq!(
        r.out(),
        format!("{a} {b} {}\n{a}", base.display()),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn pushd_with_no_operand_and_nothing_pushed_fails() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "pushd; echo \"$?\"");
    assert_eq!(r.out(), "1");
    assert!(!r.stderr.is_empty());
}

#[test]
fn pushd_rotates_to_the_named_entry() {
    let (_dir, base) = scratch();
    let r = run_in(
        &base,
        "mkdir a b; pushd a >/dev/null; pushd ../b >/dev/null; pushd +2 >/dev/null; pwd",
    );
    assert_eq!(r.out(), base.display().to_string(), "stderr: {}", r.stderr);
}

#[test]
fn a_negative_rotation_counts_from_the_bottom() {
    let (_dir, base) = scratch();
    // `-0` is the oldest entry: the directory the shell started in.
    let r = run_in(
        &base,
        "mkdir a b; pushd a >/dev/null; pushd ../b >/dev/null; pushd -0 >/dev/null; pwd",
    );
    assert_eq!(r.out(), base.display().to_string(), "stderr: {}", r.stderr);
}

#[test]
fn an_out_of_range_rotation_leaves_the_stack_alone() {
    let (_dir, base) = scratch();
    let r = run_in(
        &base,
        "mkdir a; pushd a >/dev/null; pushd +9; echo \"$?\"; dirs",
    );
    assert_eq!(
        r.out(),
        format!("1\n{}/a {}", base.display(), base.display())
    );
}

#[test]
fn popd_sets_oldpwd_so_a_later_cd_dash_comes_back() {
    let (_dir, base) = scratch();
    let r = run_in(
        &base,
        "mkdir a b; pushd a >/dev/null; pushd ../b >/dev/null; popd >/dev/null; cd - >/dev/null; pwd",
    );
    assert_eq!(
        r.out(),
        format!("{}/b", base.display()),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn popd_can_drop_an_entry_without_moving() {
    let (_dir, base) = scratch();
    let r = run_in(
        &base,
        "mkdir a b; pushd a >/dev/null; pushd ../b >/dev/null; popd +1 >/dev/null; pwd; dirs",
    );
    let b = format!("{}/b", base.display());
    assert_eq!(
        r.out(),
        format!("{b}\n{b} {}", base.display()),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn popd_on_an_empty_stack_fails() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "popd; echo \"$?\"");
    assert_eq!(r.out(), "1");
    assert!(!r.stderr.is_empty());
}

#[test]
fn dirs_v_numbers_the_entries() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir a; pushd a >/dev/null; dirs -v");
    assert_eq!(
        r.out(),
        format!(" 0  {}/a\n 1  {}", base.display(), base.display())
    );
}

#[test]
fn dirs_c_clears_everything_but_the_current_directory() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir a; pushd a >/dev/null; dirs -c; dirs");
    assert_eq!(r.out(), format!("{}/a", base.display()));
}

#[test]
fn dirs_takes_an_index() {
    let (_dir, base) = scratch();
    let r = run_in(&base, "mkdir a; pushd a >/dev/null; dirs +1; dirs -1");
    assert_eq!(r.out(), format!("{}\n{}/a", base.display(), base.display()));
}
