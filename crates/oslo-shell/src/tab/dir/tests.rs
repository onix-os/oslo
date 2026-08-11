use super::*;
use crate::tab::scratch;
use std::os::unix::fs::{PermissionsExt, symlink};

/// The ordinary case: it is not there, so it is made, at the mode it must have.
#[test]
fn a_missing_directory_is_created_private() {
    let (_scratch, _lock) = scratch();
    let fd = open_checked().expect("created");
    drop(fd);

    let made = path();
    let mode = std::fs::metadata(&made).expect("stat").permissions().mode();
    assert_eq!(mode & 0o777, 0o700, "{made:?} must be private");
}

/// Twice is the same as once.
#[test]
fn opening_an_existing_directory_is_fine() {
    let (_scratch, _lock) = scratch();
    open_checked().expect("first");
    open_checked().expect("second");
}

/// **The check this module exists for.** `/tmp` is world-writable, so a directory somebody else can
/// read is a directory that must not be used, whoever made it.
#[test]
fn a_readable_directory_is_refused() {
    let (_scratch, _lock) = scratch();
    let dir = path();
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let err = open_checked().expect_err("a 0755 tab directory must be refused");
    assert!(err.to_string().contains("755"), "{err}");
}

/// A symlink where the directory should be, which is the classic `/tmp` attack: point it at
/// somewhere interesting and let the victim write there.
#[test]
fn a_symlink_is_refused() {
    let (scratch_dir, _lock) = scratch();
    let elsewhere = scratch_dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    symlink(&elsewhere, path()).expect("symlink");

    open_checked().expect_err("a symlink must be refused");
    // The errno differs — `O_NOFOLLOW` answers ELOOP, and with `O_DIRECTORY` Linux may answer
    // ENOTDIR instead — so what is asserted is the thing that matters: nothing was reached through
    // the link. A test that pinned the errno would fail on a kernel that chose the other one while
    // being just as safe.
    assert_eq!(
        std::fs::read_dir(&elsewhere).expect("readable").count(),
        0,
        "the symlink target was written through"
    );
}

/// A plain file wearing the directory's name.
#[test]
fn a_file_is_refused() {
    let (_scratch, _lock) = scratch();
    std::fs::write(path(), b"not a directory").expect("write");
    assert!(open_checked().is_err(), "a file must be refused");
}

/// The default location, when nothing overrides it.
#[test]
fn the_default_is_under_tmp_and_per_user() {
    // Takes the shared lock, then clears what it set: this is the one test that wants the variable
    // *absent*, and a lock of its own would not exclude anything, since the variable is the process's
    // and not this module's.
    let (_scratch, _lock) = scratch();
    // SAFETY: the shared lock is held, and it is what every reader of the variable takes.
    unsafe { std::env::remove_var("OSLO_TAB_DIR") };

    let dir = path();
    let uid = nix::unistd::getuid().as_raw();
    assert_eq!(dir, PathBuf::from(format!("/tmp/oslo-{uid}/tab")));
}
