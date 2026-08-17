//! `rm -r` cannot be redirected out of the tree it was pointed at.
//!
//! # The attack
//!
//! A walk that works by path re-resolves every component on every call. Between deciding that
//! `tree/sub` is a directory and removing something inside it, anyone who can write to `tree` may
//! replace `sub` with a symlink; the next `remove_file("tree/sub/f")` then follows the link and
//! deletes a file that was never part of the tree. It is the oldest race there is, and the reason
//! `std::fs::remove_dir_all` and GNU's `rm` both traverse with `openat`/`unlinkat` instead.
//!
//! oslo's walk was path-based when it was first written, and this test failed against it — the
//! file outside the tree was deleted on the first attempt.
//!
//! # Why this is not a flaky timing test
//!
//! The swap does not have to win a race: `rm -ri` stops and *waits* for an answer before each
//! removal, so the test reads stderr until the prompt appears, does the swap while `rm` is blocked
//! on `read`, and only then answers. The window is held open by the program under test.

mod common;

use common::oslo_bin;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// Read from `stream` until `marker` has been seen, or give up after a few seconds.
///
/// Byte at a time, because the thing being waited for is a *prompt* — it has no trailing newline,
/// so any line-buffered read would block until the answer it is waiting for had already been sent.
/// Bounded so a shell that stops prompting ends the read rather than the test run; the stream
/// closing when the child exits is the ordinary way out.
const MOST_A_PROMPT_CAN_BE: usize = 64 * 1024;

fn read_until(stream: &mut impl Read, marker: &str) -> String {
    let mut seen = String::new();
    let mut byte = [0u8; 1];
    while seen.len() < MOST_A_PROMPT_CAN_BE {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => seen.push(byte[0] as char),
        }
        if seen.contains(marker) {
            break;
        }
    }
    seen
}

#[test]
fn a_directory_swapped_for_a_symlink_mid_walk_cannot_redirect_the_removal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = dir.path().join("tree");
    let sub = tree.join("sub");
    std::fs::create_dir_all(&sub).expect("create tree/sub");
    std::fs::write(sub.join("f"), "inside the tree").expect("write tree/sub/f");

    let precious = dir.path().join("precious");
    std::fs::create_dir(&precious).expect("create precious");
    let bystander = precious.join("f");
    std::fs::write(&bystander, "MUST SURVIVE").expect("write precious/f");

    let mut child = Command::new(oslo_bin())
        .arg("-c")
        .arg("rm -ri tree")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("ENV")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn oslo");

    let mut input = child.stdin.take().expect("stdin");
    let mut errors = child.stderr.take().expect("stderr");

    // Descend into `tree`, then into `tree/sub`, so the walk is holding `sub` open and is about to
    // deal with the file inside it.
    read_until(&mut errors, "descend into directory 'tree'?");
    writeln!(input, "y").expect("answer");
    read_until(&mut errors, "descend into directory 'tree/sub'?");
    writeln!(input, "y").expect("answer");

    // `rm` is now blocked on the prompt for `tree/sub/f`. Swap the directory it is standing in for
    // a symlink pointing at somewhere it was never told to touch.
    read_until(&mut errors, "remove regular file 'tree/sub/f'?");
    std::fs::remove_file(sub.join("f")).expect("clear sub");
    std::fs::remove_dir(&sub).expect("remove the real sub");
    std::os::unix::fs::symlink(&precious, &sub).expect("plant the symlink");

    // Now let it proceed. A path-based walk resolves `tree/sub/f` through the new link.
    writeln!(input, "y").expect("answer");
    writeln!(input, "y").expect("answer");
    writeln!(input, "y").expect("answer");
    drop(input);

    let _ = child.wait().expect("wait");

    assert!(
        bystander.exists(),
        "rm followed a symlink swapped in mid-walk and deleted a file outside the tree"
    );
    assert_eq!(
        std::fs::read_to_string(&bystander).expect("read the bystander"),
        "MUST SURVIVE"
    );
}

/// The same property without the race: a symlink to a directory is unlinked, never walked into.
#[test]
fn a_symlink_planted_before_the_walk_is_not_followed_either() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(&tree).expect("create tree");
    let precious = dir.path().join("precious");
    std::fs::create_dir(&precious).expect("create precious");
    let bystander = precious.join("f");
    std::fs::write(&bystander, "MUST SURVIVE").expect("write precious/f");
    std::os::unix::fs::symlink(&precious, tree.join("link")).expect("symlink");

    let out = Command::new(oslo_bin())
        .arg("-c")
        .arg("rm -rf tree")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");

    assert!(
        out.status.success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!tree.exists(), "the tree should be gone");
    assert!(bystander.exists(), "rm walked through the symlink");
}

/// A name that is not valid UTF-8 is still removed — the lossy conversion is for the *message*.
#[test]
fn a_filename_that_is_not_utf8_is_still_removed() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let tree = dir.path().join("tree");
    std::fs::create_dir(&tree).expect("create tree");
    // A lone 0xFF is not valid UTF-8 anywhere, and is a perfectly legal filename byte.
    let awkward = tree.join(OsStr::from_bytes(b"bad\xffname"));
    std::fs::write(&awkward, "x").expect("write the awkward name");
    assert!(awkward.exists());

    let out = Command::new(oslo_bin())
        .arg("-c")
        .arg("rm -rf tree")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");

    assert!(
        out.status.success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!tree.exists(), "a non-UTF-8 name stopped the walk");
}

/// `-v` names each entry as it goes, rather than the tree as a whole.
#[test]
fn verbose_names_every_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(tree.join("sub")).expect("create");
    std::fs::write(tree.join("sub/deep"), "x").expect("write");
    std::fs::write(tree.join("top"), "x").expect("write");

    let out = Command::new(oslo_bin())
        .arg("-c")
        .arg("rm -rv tree")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");

    let said = String::from_utf8_lossy(&out.stdout);
    let mut lines: Vec<&str> = said.lines().collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        vec![
            "removed 'tree/sub/deep'",
            "removed 'tree/top'",
            "removed directory 'tree'",
            "removed directory 'tree/sub'",
        ],
        "{said}"
    );
}

/// Sanity: the helper reads a prompt that has no newline after it.
#[test]
fn the_reader_stops_on_an_unterminated_prompt() {
    let mut source = BufReader::new("descend into directory 'x'? ".as_bytes());
    let seen = read_until(&mut source, "directory 'x'?");
    assert!(seen.contains("directory 'x'?"));
    assert!(source.fill_buf().is_ok());
}

/// The binary under test is the one this crate builds.
#[test]
fn the_binary_being_tested_exists() {
    assert!(Path::new(&oslo_bin()).exists(), "{:?}", oslo_bin());
}
