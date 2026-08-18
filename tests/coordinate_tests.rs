//! Stream coordinates, driven through the real binary.
//!
//! The unit tests cover the grammar and the selection. What only an end-to-end run can show is the
//! wiring: that a pipeline containing a coordinate runs its stages one at a time and threads the
//! text between them, that a value reaches the command as *one argument*, and — the part with the
//! most to lose — that a pipeline with no coordinate in it is completely untouched.

mod common;

use common::oslo_bin;
use std::process::Command;

/// Run `line` through `-c` in a directory holding a small fixture.
#[track_caller]
fn shell(line: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("hosts.txt"),
        "web-01  10.0.0.1  nginx\nweb-02  10.0.0.2  apache\ndb-01   10.0.0.9  postgres\n",
    )
    .expect("fixture");
    std::fs::write(dir.path().join("spaced.txt"), "my file.txt  100\n").expect("fixture");
    std::fs::write(dir.path().join("glob.txt"), "*.txt\n").expect("fixture");
    let out = Command::new(oslo_bin())
        .arg("-c")
        .arg(line)
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("PATH", "/usr/bin:/bin")
        .env_remove("ENV")
        .output()
        .expect("spawn oslo");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text.trim_end().to_string()
}

/// A line, a word, and the whole of a line.
#[test]
fn a_coordinate_reads_the_upstream() {
    assert_eq!(shell("cat hosts.txt | echo {0:0}"), "web-01");
    assert_eq!(shell("cat hosts.txt | echo {0:1}"), "10.0.0.1");
    assert_eq!(
        shell("cat hosts.txt | echo {1}"),
        "web-02  10.0.0.2  apache"
    );
    assert_eq!(shell("cat hosts.txt | echo {-1:0}"), "db-01");
    assert_eq!(shell("cat hosts.txt | echo {-1:-1}"), "postgres");
}

/// **`*` yields many arguments to one command**, the way `"$@"` does — not many commands.
#[test]
fn a_star_becomes_many_arguments() {
    assert_eq!(shell("cat hosts.txt | echo {*:0}"), "web-01 web-02 db-01");
    assert_eq!(
        shell("cat hosts.txt | echo {*:1}"),
        "10.0.0.1 10.0.0.2 10.0.0.9"
    );
    // One `echo`, so one line of output — three commands would print three.
    assert_eq!(shell("cat hosts.txt | echo {*:0}").lines().count(), 1);
}

/// Text around a coordinate keeps it one word, because `host-{0:0}.lan` means nothing otherwise.
#[test]
fn text_around_a_coordinate_keeps_one_word() {
    assert_eq!(
        shell("cat hosts.txt | echo host-{0:0}.lan"),
        "host-web-01.lan"
    );
}

/// **Three dimensions reach past a stage.** This is the whole point of the stream axis: `{1:…}`
/// steps back past the stage feeding this one.
#[test]
fn a_third_dimension_reaches_back_past_a_stage() {
    // `grep db` leaves one line, so `{0:0}` is `db-01` — and `{1:0:0}` is what `cat` printed.
    assert_eq!(shell("cat hosts.txt | grep db | echo {0:0}"), "db-01");
    assert_eq!(shell("cat hosts.txt | grep db | echo {1:0:0}"), "web-01");
}

/// **A value is one argument.** A line holding spaces arrives whole; only an explicit word
/// dimension splits it. This is the difference between one filename and three.
#[test]
fn a_value_is_one_argument_unless_words_were_asked_for() {
    assert_eq!(
        shell(r"cat spaced.txt | printf '[%s]\n' {0}"),
        "[my file.txt  100]"
    );
    assert_eq!(
        shell(r"cat spaced.txt | printf '[%s]\n' {0:*}"),
        "[my]\n[file.txt]\n[100]"
    );
}

/// **A glob in the data is data.** The fixture directory holds several `.txt` files, so a
/// substituted `*.txt` that was re-globbed would come back as three names.
#[test]
fn a_substituted_glob_does_not_glob() {
    assert_eq!(shell("cat glob.txt | echo {0:0}"), "*.txt");
}

/// **A quoted coordinate is text.** Every other expansion offers a way to write the characters
/// themselves and so does this one.
#[test]
fn a_quoted_coordinate_is_left_alone() {
    assert_eq!(shell("cat hosts.txt | echo '{0:0}'"), "{0:0}");
    assert_eq!(shell("cat hosts.txt | echo \"{0:0}\""), "{0:0}");
}

/// **Nothing else changes.** The gate is the load-bearing part of this feature: a pipeline with no
/// coordinate must run down the path it always did, concurrently, capturing nothing.
#[test]
fn an_ordinary_pipeline_is_untouched() {
    assert_eq!(shell("seq 1 5 | head -2"), "1\n2");
    assert_eq!(shell("echo hi | cat"), "hi");
    // Brace expansion still owns its syntax.
    assert_eq!(shell("echo {a,b}"), "a b");
    assert_eq!(shell("echo {0..2}"), "0 1 2");
    // And a brace group that is not a coordinate is left for it.
    assert_eq!(shell("cat hosts.txt | echo {a,b}"), "a b");
}

/// `PIPESTATUS` still reports every stage, including a stage that failed.
#[test]
fn every_stage_still_reports_its_status() {
    assert_eq!(
        shell(r#"cat hosts.txt | echo {0:0} >/dev/null; echo "${PIPESTATUS[*]}""#),
        "0 0"
    );
    assert_eq!(
        shell(r#"false | echo {0:0} >/dev/null; echo "${PIPESTATUS[*]}""#),
        "1 0"
    );
}

/// An empty or missing selection reads as nothing rather than refusing to run.
#[test]
fn a_selection_that_finds_nothing_still_runs() {
    assert_eq!(shell("cat hosts.txt | echo [{9:9}]"), "[]");
    assert_eq!(shell("cat hosts.txt | echo done {9}"), "done");
}
