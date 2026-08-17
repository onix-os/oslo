//! `oslo.fs` and `oslo.path`.
//!
//! The point of these namespaces is that a script never has to parse another program's output.
//! `oslo.fs.ls` answers with fields, so a filename holding a space, a newline or a `-` is just a
//! `name`, where `ls -l | awk '{print $9}'` gets each of those wrong in a different way.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

/// Run a Lua chunk in a fresh directory and return its stdout, trimmed.
#[track_caller]
fn lua(script: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("case.lua");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(oslo_bin())
        .arg(&path)
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("TMPDIR", dir.path())
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

#[test]
fn read_and_write_round_trip() {
    let out = lua(r#"
        assert(oslo.fs.write("f.txt", "hello\n"))
        print(oslo.fs.read("f.txt") == "hello\n")
        assert(oslo.fs.append("f.txt", "again\n"))
        print(oslo.fs.read("f.txt"))
    "#);
    assert_eq!(out, "true\nhello\nagain");
}

#[test]
fn a_missing_file_answers_rather_than_raising() {
    // `nil, message`, the shape `io.open` uses. A missing file is a condition a script handles,
    // not a bug in it.
    let out = lua(r#"
        local content, err = oslo.fs.read("nope.txt")
        print(content, err ~= nil, err:find("nope.txt") ~= nil)
    "#);
    assert_eq!(out, "nil\ttrue\ttrue");
}

#[test]
fn lines_does_not_invent_a_trailing_empty_one() {
    let out = lua(r#"
        local function count(path)
          local n = 0
          for _ in oslo.fs.lines(path) do n = n + 1 end
          return n
        end
        oslo.fs.write("a", "one\ntwo\n")
        oslo.fs.write("b", "one\ntwo")
        oslo.fs.write("c", "")
        print(count("a"), count("b"), count("c"))
    "#);
    assert_eq!(out, "2\t2\t0");
}

/// **The file is read as it is asked for**, which is the point of the iterator: a loop that stops
/// at the first line has not read the rest, and `<close>` shuts the descriptor.
#[test]
fn lines_holds_the_file_open_and_lets_it_go() {
    let out = lua(r#"
        oslo.fs.write("big", "one\ntwo\nthree\n")
        local first
        do
          local f <close> = oslo.fs.lines("big")
          first = f()
        end
        print(first)
        -- Reading a file that is not there is a message, before anything is read.
        local it, why = oslo.fs.lines("nope")
        print(it == nil, why ~= nil)
    "#);
    assert_eq!(out, "one\ntrue\ttrue");
}

#[test]
fn ls_answers_with_fields_not_text() {
    let out = lua(r#"
        oslo.fs.write("beta.txt", "12345")
        oslo.fs.mkdir("alpha")
        local entries = oslo.fs.ls(".")
        for _, e in ipairs(entries) do
            if e.name ~= "case.lua" then print(e.name, e.type, e.size) end
        end
    "#);
    // Sorted by name, so `alpha` comes before `beta.txt` however the filesystem stored them.
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0].split('\t').next(), Some("alpha"));
    assert!(lines[0].contains("directory"), "{out}");
    assert_eq!(lines[1], "beta.txt\tfile\t5", "{out}");
}

/// A filename that would break every text-parsing approach must be an ordinary `name` here.
#[test]
fn a_hostile_filename_is_just_a_field() {
    let out = lua(r#"
        local weird = "a file  with -dashes and 'quotes'"
        oslo.fs.write(weird, "x")
        for _, e in ipairs(oslo.fs.ls(".")) do
            if e.name == weird then print("found", e.size) end
        end
    "#);
    assert_eq!(out, "found\t1");
}

#[test]
fn mkdir_is_always_recursive_and_succeeds_twice() {
    let out = lua(r#"
        assert(oslo.fs.mkdir("a/b/c"))
        -- Again: a script wants the directory to exist afterwards, not to fail because it does.
        assert(oslo.fs.mkdir("a/b/c"))
        print(oslo.fs.stat("a/b/c").type)
    "#);
    assert_eq!(out, "directory");
}

#[test]
fn remove_needs_asking_before_it_takes_a_tree() {
    let out = lua(r#"
        oslo.fs.mkdir("tree/inner")
        local ok, err = oslo.fs.remove("tree")
        print(ok, err ~= nil)
        print(oslo.fs.remove("tree", true))
        print(oslo.fs.exists("tree"))
    "#);
    assert_eq!(out, "nil\ttrue\ntrue\nfalse");
}

/// Removing a symlink removes the link, never what it points at.
#[test]
fn remove_does_not_follow_a_symlink() {
    let out = lua(r#"
        oslo.fs.write("target", "precious")
        assert(oslo.fs.symlink("target", "link"))
        assert(oslo.fs.remove("link"))
        print(oslo.fs.exists("link"), oslo.fs.exists("target"))
    "#);
    assert_eq!(out, "false\ttrue");
}

#[test]
fn stat_reports_the_mode_without_the_type_bits() {
    let out = lua(r#"
        oslo.fs.write("x", "")
        assert(oslo.fs.chmod("x", 0x1ED))   -- 0755
        print(string.format("%o", oslo.fs.stat("x").mode))
    "#);
    assert_eq!(out, "755");
}

#[test]
fn walk_lists_a_tree_without_following_links_out_of_it() {
    let out = lua(r#"
        oslo.fs.mkdir("d/inner")
        oslo.fs.write("d/inner/f", "")
        -- A link back to the top would make a following walk never finish.
        oslo.fs.symlink("..", "d/inner/up")
        local found = {}
        for path in oslo.fs.walk("d") do found[#found + 1] = path end
        table.sort(found)
        print(#found, table.concat(found, " "))
    "#);
    assert_eq!(out, "3\td/inner d/inner/f d/inner/up");
}

/// **The walk is lazy**, which is the whole reason it answers an iterator: a loop that stops after
/// one entry has read one directory, not the tree. `<close>` lets go of the descriptors it opened
/// on the way down.
#[test]
fn walk_stops_when_the_loop_does() {
    let out = lua(r#"
        oslo.fs.mkdir("d/inner/deeper")
        oslo.fs.write("d/inner/deeper/f", "")
        local seen = 0
        do
          local tree <close> = oslo.fs.walk("d")
          for _ in tree do seen = seen + 1; break end
        end
        print(seen)
        -- A directory that is not there is a message, before anything is walked.
        local it, why = oslo.fs.walk("d/nope")
        print(it == nil, why ~= nil)
    "#);
    assert_eq!(out, "1\ntrue\ttrue");
}

#[test]
fn glob_uses_the_shells_own_matcher() {
    let out = lua(r#"
        oslo.fs.write("one.conf", "")
        oslo.fs.write("two.conf", "")
        oslo.fs.write("three.txt", "")
        local found = oslo.fs.glob("*.conf")
        table.sort(found)
        print(#found, table.concat(found, ","))
        -- No matches is an empty table, never the pattern handed back.
        print(#oslo.fs.glob("*.nothing"))
    "#);
    assert_eq!(out, "2\tone.conf,two.conf\n0");
}

#[test]
fn mktemp_creates_the_file_rather_than_naming_one() {
    // Returning a name for the caller to open is the classic temp-file race: between the answer
    // and the open, anything can take the name.
    let out = lua(r#"
        local a = oslo.fs.mktemp("t")
        local b = oslo.fs.mktemp("t")
        print(a ~= b, oslo.fs.exists(a), oslo.fs.exists(b))
        print(oslo.fs.stat(oslo.fs.mktempdir("d").path).type)
    "#);
    assert_eq!(out, "true\ttrue\ttrue\ndirectory");
}

/// **A temporary directory is the one thing in `oslo.fs` with a lifetime**, so it answers a handle
/// rather than a path: `<close>` removes it at the end of the block, and `tostring` is the path so
/// it still reads as one.
#[test]
fn a_temporary_directory_is_removed_at_the_end_of_its_block() {
    let out = lua(r#"
        local path
        do
          local tmp <close> = oslo.fs.mktempdir("scope")
          path = tmp.path
          oslo.fs.write(tmp.path .. "/x", "hi")
          print(tostring(tmp) == path, oslo.fs.exists(path .. "/x"))
        end
        print(oslo.fs.exists(path))
    "#);
    assert_eq!(out, "true\ttrue\nfalse");
}

#[test]
fn path_splits_a_name_into_its_parts() {
    let out = lua(r#"
        print(oslo.path.parent("/a/b/c.txt"), oslo.path.name("/a/b/c.txt"))
        print(oslo.path.stem("/a/b/c.txt"), oslo.path.ext("/a/b/c.txt"))
        -- A dotfile is a name, not an extension of an empty stem.
        print(oslo.path.stem(".bashrc"), oslo.path.ext(".bashrc"))
        -- Nothing above the root, and nothing above a bare name.
        print(oslo.path.parent("/"), oslo.path.parent("x"))
    "#);
    assert_eq!(out, "/a/b\tc.txt\nc\ttxt\n.bashrc\tnil\nnil\tnil");
}

#[test]
fn join_lets_an_absolute_component_restart_the_path() {
    let out = lua(r#"
        print(oslo.path.join("a", "b", "c"))
        print(oslo.path.join("/etc", "/tmp"))
        print(oslo.path.join("a", 1))
    "#);
    assert_eq!(out, "a/b/c\n/tmp\na/1");
}

#[test]
fn normalize_is_lexical_and_never_touches_the_disk() {
    let out = lua(r#"
        print(oslo.path.normalize("a/./b/../c"))
        print(oslo.path.normalize("/a/b/../../c"))
        -- A relative `..` at the front names a real, different place.
        print(oslo.path.normalize("../a"))
        print(oslo.path.normalize("a/.."))
        -- And it answers for a path that does not exist, which realpath cannot.
        print(oslo.path.normalize("/no/such/dir/../file"))
    "#);
    assert_eq!(out, "a/c\n/c\n../a\n.\n/no/such/file");
}

#[test]
fn path_helpers_cover_the_rest_of_the_shape() {
    let out = lua(r#"
        print(oslo.path.is_absolute("/a"), oslo.path.is_absolute("a"))
        print(table.concat(oslo.path.split("/a/b/c"), "|"))
        print(oslo.path.relative_to("/a/b/c", "/a"))
        -- Not under the base is nil, not the original path pretending to be an answer.
        print(oslo.path.relative_to("/x/y", "/a"))
        -- `~` comes from $HOME, which the harness points at the case's own directory.
        print(oslo.path.expand("~/x"):sub(-2), oslo.path.expand("~notauser/x"))
    "#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "true\tfalse");
    assert_eq!(lines[1], "/|a|b|c");
    assert_eq!(lines[2], "b/c");
    assert_eq!(lines[3], "nil");
    // `~user` is left alone rather than guessed at.
    assert_eq!(lines[4], "/x\t~notauser/x");
}

#[test]
fn realpath_resolves_links_where_normalize_does_not() {
    let out = lua(r#"
        oslo.fs.mkdir("real")
        oslo.fs.symlink("real", "link")
        local resolved = oslo.fs.realpath("link")
        print(oslo.path.name(resolved))
        print(oslo.fs.readlink("link"))
    "#);
    assert_eq!(out, "real\nreal");
}

/// **`touch` is both halves**: it creates what is missing and moves the timestamp of what is not.
/// Written by hand as `append(path, "")` it does only the first, which is the half nobody wanted.
#[test]
fn touch_creates_what_is_missing_and_dates_what_is_not() {
    let out = lua(r#"
        print(oslo.fs.exists("t"), oslo.fs.touch("t"), oslo.fs.exists("t"), oslo.fs.stat("t").size)
        -- An existing file keeps its contents.
        oslo.fs.write("t", "kept")
        oslo.fs.touch("t")
        print(oslo.fs.read("t"), oslo.fs.stat("t").size)
        -- Under a directory that is not there, a message rather than a raise.
        local ok, err = oslo.fs.touch("nope/t")
        print(ok, err.kind)
    "#);
    assert_eq!(out, "false\ttrue\ttrue\t0\nkept\t4\nnil\tnot-found");
}

/// **What `du -s --apparent-size` answers**, and a symlink is counted as the link it is: its size
/// is the length of its target, and it is never followed. Following one is how a tree containing a
/// link to its own parent never finishes.
#[test]
fn usage_adds_up_a_tree_without_following_links() {
    let out = lua(r#"
        oslo.fs.mkdir("tree/inner")
        oslo.fs.write("tree/a", "12345")           -- 5
        oslo.fs.write("tree/inner/b", "1234567890") -- 10
        oslo.fs.symlink("..", "tree/inner/up")      -- 2, the length of ".."
        local u = oslo.fs.usage("tree")
        print(u.bytes, u.files, u.dirs, u.unreadable)
        local missing, err = oslo.fs.usage("tree/nope")
        print(missing, err.kind)
    "#);
    assert_eq!(out, "17\t3\t1\t0\nnil\tnot-found");
}

/// **A subdirectory it cannot read is counted, not fatal.** Stopping on the first `EACCES` made
/// `usage("/etc")` answer nil for anybody who is not root — one unreadable directory losing the
/// whole total. `unreadable` is how a caller tells a complete answer from a floor.
#[test]
fn usage_carries_on_past_what_it_cannot_read_and_says_so() {
    let out = lua(r#"
        oslo.fs.mkdir("tree/open")
        oslo.fs.mkdir("tree/shut")
        oslo.fs.write("tree/open/a", "12345")
        oslo.fs.write("tree/shut/hidden", "1234567890")
        assert(oslo.fs.chmod("tree/shut", 0))     -- unreadable, unenterable

        local u = oslo.fs.usage("tree")
        print(u.bytes, u.files, u.unreadable)
        oslo.fs.chmod("tree/shut", 0x1C0)         -- 0700, so the tempdir can be cleaned up
    "#);
    // The readable half is counted, the shut one is reported rather than silently missing.
    assert_eq!(out, "5\t1\t1");
}
