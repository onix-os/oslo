use super::*;

fn words(line: &str) -> Vec<String> {
    line.split(' ').map(str::to_string).collect()
}

fn out(line: &str) -> Vec<String> {
    let (status, rows) = run(&words(line), None, None).expect("path always answers");
    assert_eq!(status, 0, "{line}");
    rows.expect("a run that succeeded produced rows")
        .iter()
        .map(string_of)
        .collect()
}

fn status(line: &str) -> i32 {
    run(&words(line), None, None)
        .expect("path always answers")
        .0
}

/// A trailing slash says "a directory", not "ends in nothing".
#[test]
fn basename_and_dirname_survive_a_trailing_slash() {
    assert_eq!(out("path basename /a/b/c.rs"), ["c.rs"]);
    assert_eq!(out("path basename /a/b/"), ["b"]);
    assert_eq!(out("path basename /"), ["/"]);
    assert_eq!(out("path dirname /a/b/c.rs"), ["/a/b"]);
    assert_eq!(out("path dirname /a"), ["/"]);
    assert_eq!(out("path dirname bare"), ["."]);
}

/// A dotfile is a name beginning with a dot, not a name that is all extension.
#[test]
fn extension_has_no_dot_and_a_dotfile_has_none_at_all() {
    assert_eq!(out("path extension a.rs"), ["rs"]);
    assert_eq!(out("path extension /a/b.tar.gz"), ["gz"]);
    assert_eq!(out("path extension .bashrc"), [""]);
    assert_eq!(out("path extension plain"), [""]);
}

#[test]
fn change_extension_takes_it_with_or_without_the_dot() {
    assert_eq!(out("path change-extension rs /a/b.c"), ["/a/b.rs"]);
    assert_eq!(out("path change-extension .rs /a/b.c"), ["/a/b.rs"]);
    assert_eq!(out("path change-extension rs /a/plain"), ["/a/plain.rs"]);
    assert_eq!(out("path change-extension  /a/b.c"), ["/a/b"]);
    // A dotfile keeps its name and gains an extension rather than losing the leading dot.
    assert_eq!(out("path change-extension bak .rc"), [".rc.bak"]);
}

/// Lexical, so a path that does not exist still has an answer.
#[test]
fn normalize_never_touches_the_disk() {
    assert_eq!(out("path normalize /a/./b/../c"), ["/a/c"]);
    assert_eq!(out("path normalize a//b/"), ["a/b"]);
    assert_eq!(out("path normalize ./"), ["."]);
    assert_eq!(out("path normalize /.."), ["/"]);
    // A relative path may climb out of itself; there is nothing above it to clamp against.
    assert_eq!(out("path normalize ../../x"), ["../../x"]);
    assert_eq!(
        out("path normalize /does/not/exist/../here"),
        ["/does/not/here"]
    );
}

/// A path the filesystem cannot answer for still gets the lexical answer.
#[test]
fn resolve_falls_back_rather_than_failing() {
    assert_eq!(out("path resolve /tmp/./nothing-here/../x"), ["/tmp/x"]);
    assert_eq!(out("path resolve /"), ["/"]);
}

#[test]
fn sort_orders_by_the_whole_path_unless_told_a_key() {
    assert_eq!(out("path sort b/1 a/2"), ["a/2", "b/1"]);
    assert_eq!(out("path sort --reverse a/2 b/1"), ["b/1", "a/2"]);
    assert_eq!(out("path sort --key basename b/1 a/2"), ["b/1", "a/2"]);
}

/// `filter` answers rows and `is` answers a status — the difference the module note is about.
#[test]
fn filter_keeps_what_is_and_is_only_says_whether() {
    assert_eq!(out("path filter -d /tmp /does-not-exist"), ["/tmp"]);
    assert_eq!(
        out("path filter --invert -d /tmp /does-not-exist"),
        ["/does-not-exist"]
    );
    assert_eq!(status("path is -d /tmp"), 0);
    assert_eq!(status("path is -f /tmp"), 1);
    // Every path, not any: one failure fails the question.
    assert_eq!(status("path is -d /tmp /does-not-exist"), 1);
    let (_, rows) = run(&words("path is -d /tmp"), None, None).unwrap();
    assert_eq!(rows.expect("is answers a status and no rows").len(), 0);
}

/// Nothing found is false — there is no file it can be true about.
#[test]
fn is_with_nothing_to_test_is_false() {
    assert_eq!(status("path is -f"), 1);
}

/// A row from `ls` is read from `name`, which is why this works with no column named.
#[test]
fn a_row_is_read_without_naming_its_column() {
    let rows = vec![Record::from_pairs([("name", Val::Str("a/b.rs".into()))])];
    let (_, answered) = run(&words("path extension"), Some(&rows), None).unwrap();
    assert_eq!(answered.unwrap().first().map(string_of).unwrap(), "rs");
}

#[test]
fn a_subcommand_that_is_not_one_is_refused() {
    assert_eq!(status("path nope x"), 2);
    assert_eq!(status("path"), 2);
    assert_eq!(status("path change-extension"), 2);
    assert_eq!(status("path sort --key size a"), 2);
    assert_eq!(status("path basename --nope x"), 2);
    assert_eq!(status("path mtime /does-not-exist"), 2);
}
