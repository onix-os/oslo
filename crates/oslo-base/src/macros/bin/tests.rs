use super::*;

/// The tests point `$OSLO_MACROS_BIN` at a temporary directory. Serialised, because the environment
/// is process-wide and libtest's threads share it — the hazard `track::universal` documents.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn script(name: &str, body: &str) -> Entry {
    Entry::new(Kind::Script, name, body)
}

#[test]
fn a_stored_script_becomes_a_file_that_can_be_run() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    // SAFETY: guarded by `lock`, and read only by the code under test.
    unsafe { std::env::set_var("OSLO_MACROS_BIN", dir.path()) };

    publish(&[script("hello", "#!/bin/sh\necho hi\n")]).expect("publish");

    let path = dir.path().join("hello");
    assert_eq!(
        std::fs::read_to_string(&path).expect("written"),
        "#!/bin/sh\necho hi\n"
    );
    let mode = std::os::unix::fs::PermissionsExt::mode(
        &std::fs::metadata(&path).expect("stat").permissions(),
    );
    assert_eq!(mode & 0o111, 0o100, "executable by its owner: {mode:o}");
    assert_eq!(mode & 0o077, 0, "and by nobody else: {mode:o}");

    unsafe { std::env::remove_var("OSLO_MACROS_BIN") };
}

/// **A script that is no longer stored stops being a file.** Otherwise removing a macro would leave
/// the copy on `$PATH` running for ever, which is the failure a derived file has.
#[test]
fn what_is_no_longer_stored_is_taken_away() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("OSLO_MACROS_BIN", dir.path()) };

    publish(&[script("one", "#!/bin/sh\n"), script("two", "#!/bin/sh\n")]).expect("publish");
    assert!(dir.path().join("two").exists());

    publish(&[script("one", "#!/bin/sh\n")]).expect("republish");
    assert!(dir.path().join("one").exists(), "still stored");
    assert!(!dir.path().join("two").exists(), "no longer stored");

    unsafe { std::env::remove_var("OSLO_MACROS_BIN") };
}

/// **Somebody else's file is not oslo's to delete**, even in oslo's own directory. The manifest is
/// what tells them apart, and the cost of being wrong is a person's script.
#[test]
fn a_file_oslo_did_not_write_is_left_alone() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("OSLO_MACROS_BIN", dir.path()) };

    std::fs::write(dir.path().join("theirs"), "#!/bin/sh\n").expect("write");
    publish(&[script("ours", "#!/bin/sh\n")]).expect("publish");
    publish(&[]).expect("republish with nothing stored");

    assert!(dir.path().join("theirs").exists(), "not oslo's to remove");
    assert!(!dir.path().join("ours").exists(), "oslo's, and gone");

    unsafe { std::env::remove_var("OSLO_MACROS_BIN") };
}

/// A function runs *in* the calling shell, so it cannot be a file and is not written as one.
#[test]
fn only_scripts_are_written_out() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("OSLO_MACROS_BIN", dir.path()) };

    publish(&[
        Entry::new(Kind::Func, "mkcd", "mkdir -p \"$1\" && cd \"$1\"\n"),
        Entry::new(Kind::Alias, "gs", "git status"),
        script("deploy", "#!/bin/sh\n"),
    ])
    .expect("publish");

    assert!(dir.path().join("deploy").exists());
    assert!(
        !dir.path().join("mkcd").exists(),
        "a function is not a file"
    );
    assert!(!dir.path().join("gs").exists());

    unsafe { std::env::remove_var("OSLO_MACROS_BIN") };
}

/// One turned off is one that does not apply, so its file goes with it.
#[test]
fn a_macro_turned_off_has_no_file() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("OSLO_MACROS_BIN", dir.path()) };

    let mut off = script("paused", "#!/bin/sh\n");
    off.active = false;
    publish(&[off]).expect("publish");
    assert!(!dir.path().join("paused").exists());

    unsafe { std::env::remove_var("OSLO_MACROS_BIN") };
}

/// **A directory belongs to the store that filled it**, and a second store may not empty it.
///
/// This is not a hypothetical. Pointing `$XDG_DATA_HOME` at a scratch store and running
/// `oslo macros publish` deleted seventy-two scripts out of a real `~/.local/sbin`, because the
/// store had moved and the script directory had not — it is `$OSLO_MACROS_BIN` or `~/.local/sbin`
/// and nothing about it follows the store. The manifest now says whose the directory is.
#[test]
fn a_second_store_does_not_empty_the_first_ones_directory() {
    let _guard = lock();
    let bin = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("OSLO_MACROS_BIN", bin.path()) };

    // A directory another store filled: its manifest says so, and its script is in it. Written by
    // hand rather than by pointing `$XDG_DATA_HOME` somewhere — that is process-wide, and moving it
    // under other tests is what `track::universal` warns about.
    std::fs::write(bin.path().join("deploy"), "#!/bin/sh\necho theirs\n").expect("their script");
    std::fs::write(
        bin.path().join(".oslo"),
        "store /somewhere/else/oslo/macros\ndeploy\n",
    )
    .expect("their manifest");

    let refused = publish(&[]);
    assert!(refused.is_err(), "a foreign store was allowed to publish");
    let said = refused.unwrap_err();
    assert!(said.contains("/somewhere/else"), "it names whose: {said}");
    assert!(
        said.contains("OSLO_MACROS_BIN"),
        "a way out is named: {said}"
    );
    assert!(
        bin.path().join("deploy").exists(),
        "the other store's script was deleted"
    );

    // A directory with no owner at all is nobody's, and may be taken: that is every directory
    // written before this line existed.
    std::fs::write(bin.path().join(".oslo"), "deploy\n").expect("an old manifest");
    publish(&[script("ours", "#!/bin/sh\n")]).expect("an unclaimed directory");
    assert!(bin.path().join("ours").exists());

    unsafe { std::env::remove_var("OSLO_MACROS_BIN") };
}

/// What the command resolver asks, to know it is looking at oslo's own output.
///
/// **Written by oslo, not merely sitting where oslo writes.** A file somebody put in the directory
/// by hand is theirs: bash runs it, and a shell that answered "command not found" for its own
/// directory would be the only thing on the system that could not see it.
#[test]
fn ours_means_this_module_wrote_it() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    unsafe { std::env::set_var("OSLO_MACROS_BIN", dir.path()) };
    publish(&[script("git-rel", "#!/bin/sh\n")]).expect("publish");

    assert!(is_ours(&dir.path().join("git-rel")));
    assert!(
        !is_ours(&dir.path().join("hand-written")),
        "a file oslo never wrote is not oslo's to hide"
    );
    assert!(!is_ours(std::path::Path::new("/usr/bin/git-rel")));
    assert!(!is_ours(&dir.path().join("nested/git-rel")));

    unsafe { std::env::remove_var("OSLO_MACROS_BIN") };
}
