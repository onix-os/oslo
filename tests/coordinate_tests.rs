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

/// **A negative stream is the previous command *line*** — its words are the command and its
/// arguments.
///
/// This is what the original ask spelled `$ARG_PREV_n`, folded into the one grammar. It costs
/// nothing: the line is already known, unlike a command's *output*, which would mean standing
/// between the command and the terminal and turning `isatty` false for everything.
///
/// Driven through `-c` with two commands, because `-c` runs a list in one shell.
#[test]
fn a_negative_stream_reads_the_previous_command_line() {
    // `-c` is a single command list, so `remember_prompt` never runs — the previous-prompt ring is
    // the interactive loop's. What can be checked here is that the syntax parses and reads empty
    // rather than misbehaving, which is the failure that would matter.
    assert_eq!(shell("echo one two; echo [{-1:0:1}]"), "one two\n[]");
}

/// **Two dimensions never reach a stream.** `{-1:0}` is line −1, word 0 of *this* input — not the
/// previous command. Reaching a stream takes all three, and the trailing colon is how a whole line
/// is asked for: `{-1:0:}`.
#[test]
fn two_dimensions_do_not_reach_a_stream() {
    // Line -1 of the upstream, word 0.
    assert_eq!(shell("cat hosts.txt | echo {-1:0}"), "db-01");
    // Three dimensions, and the last empty, is the whole of that line.
    assert_eq!(
        shell("cat hosts.txt | grep db | echo {1:-1:}"),
        "db-01   10.0.0.9  postgres"
    );
}

/// **Every coordinate in a word is replaced, not just the first.**
///
/// `{0:0}-{1:0}` used to come back as `alpha-{1:0}`: the scan found one coordinate and stopped.
#[test]
fn every_coordinate_in_a_word_is_replaced() {
    assert_eq!(shell("cat hosts.txt | echo {0:0}-{1:0}"), "web-01-web-02");
    assert_eq!(
        shell("cat hosts.txt | echo {0:0}{1:0}{-1:0}"),
        "web-01web-02db-01"
    );
    assert_eq!(
        shell("cat hosts.txt | echo pre-{0:0}-mid-{-1:0}-post"),
        "pre-web-01-mid-db-01-post"
    );
    // And a brace group in front of one does not hide it.
    assert_eq!(shell("cat hosts.txt | echo a{b}c{0:0}"), "a{b}cweb-01");
}

/// **An endless upstream stops.**
///
/// Reading to EOF and truncating afterwards is fine for a file and fatal for a tap: `yes | echo
/// {0:0}` hung for ever, and so did `yes | head -3 | echo {0:0}` — capturing the first stage to EOF
/// defeats `head`'s early exit, because the thing draining `yes` is no longer `head`. The read is
/// bounded instead, and closing it kills the producer with `SIGPIPE` exactly as `head` would.
#[test]
fn an_endless_upstream_is_cut_off() {
    assert_eq!(shell("yes | echo {0:0}"), "y");
    assert_eq!(shell("yes | head -3 | echo {0:0}"), "y");
}

/// A malformed coordinate is left as text — no panic, no crash, and no swallowing of a brace group.
#[test]
fn a_malformed_coordinate_is_left_alone() {
    for text in [
        "{}",
        "{:::}",
        "{0:1:2:3}",
        "{--1}",
        "{-}",
        "{0:0",
        "{ 0:0 }",
        // Far past what an index can hold: refused rather than overflowing.
        "{999999999999999999999}",
    ] {
        assert_eq!(
            shell(&format!("cat hosts.txt | echo [{text}]")),
            format!("[{text}]"),
            "for {text}"
        );
    }
}

/// The shapes real text arrives in.
#[test]
fn the_edges_of_real_input() {
    // No trailing newline still has a last line.
    assert_eq!(shell(r#"printf "x y" | echo [{0:1}]"#), "[y]");
    // Nothing at all reads as nothing.
    assert_eq!(shell("true | echo [{0:0}]"), "[]");
    assert_eq!(shell(r#"printf "\n" | echo [{0}]"#), "[]");
    // A blank line in the middle is a line, and keeps the ones after it in place.
    assert_eq!(shell(r#"printf "a\n\nb\n" | echo [{1}][{2:0}]"#), "[][b]");
    // Tabs separate words, as whitespace does everywhere else in a shell.
    assert_eq!(shell(r#"printf "a\tb\n" | echo [{0:1}]"#), "[b]");
    // Leading whitespace is not a word.
    assert_eq!(shell(r#"printf "   a   b\n" | echo [{0:0}]"#), "[a]");
    // A line of only spaces has no words, but is still a line.
    assert_eq!(shell(r#"printf "   \n" | echo [{0:0}][{0}]"#), "[][   ]");
    // Non-ASCII is text like any other.
    assert_eq!(shell(r#"printf "héllo wörld\n" | echo [{0:1}]"#), "[wörld]");
}

/// A coordinate may be the command itself.
#[test]
fn a_coordinate_can_name_the_command() {
    assert_eq!(shell(r#"printf "echo\n" | {0:0} ran-it"#), "ran-it");
}

/// **`pipefail` reports the rightmost stage that failed**, and the coordinate path must use the
/// same rule as the concurrent one.
///
/// It did not: it took the last status directly, so `set -o pipefail; false | echo {0:0}` reported
/// 0 while the same pipeline without a coordinate reported 1. A status that depends on whether a
/// coordinate happens to be present is a status nobody can rely on.
#[test]
fn pipefail_is_obeyed() {
    assert_eq!(
        shell("set -o pipefail; false | echo {0:0} >/dev/null; echo rc=$?"),
        "rc=1"
    );
    assert_eq!(
        shell("set -o pipefail; echo x | false | echo {0:0} >/dev/null; echo rc=$?"),
        "rc=1"
    );
    // Without the option, only the last stage decides — the POSIX default.
    assert_eq!(shell("false | echo {0:0} >/dev/null; echo rc=$?"), "rc=0");
}

/// **Only stdout is a stream.** Standard error goes where it always went and never lands in a
/// coordinate — a diagnostic is not data.
#[test]
fn stderr_is_not_captured() {
    assert_eq!(
        shell(r#"sh -c "echo OUT; echo ERR >&2" | echo [{0:0}]"#),
        "[OUT]\nERR"
    );
    assert_eq!(shell(r#"sh -c "echo ERR >&2" | echo [{0:0}]"#), "[]\nERR");
}

/// A coordinate is rewritten afresh every time the command runs, so a loop does not answer with
/// the first iteration's text for ever. This is why the command is cloned rather than rewritten.
#[test]
fn a_loop_substitutes_each_time_around() {
    assert_eq!(
        shell(r#"for i in 1 2 3; do printf "v$i\n" | echo [{0:0}]; done"#),
        "[v1]\n[v2]\n[v3]"
    );
}

/// Shell functions work on both sides of the pipe.
#[test]
fn functions_are_ordinary_stages() {
    assert_eq!(
        shell(r#"f(){ echo "got $1"; }; printf "a b\n" | f {0:1}"#),
        "got b"
    );
    assert_eq!(shell("f(){ echo from-fn; }; f | echo {0:0}"), "from-fn");
}

/// With no upstream at all, a coordinate reads empty rather than failing.
#[test]
fn no_upstream_reads_empty() {
    assert_eq!(shell("echo [{0:0}]"), "[]");
    assert_eq!(shell("echo [{-1:0:0}]"), "[]");
}

/// Redirections, heredocs and nesting are all ordinary around a coordinate.
#[test]
fn the_usual_shell_machinery_still_works() {
    assert_eq!(shell("cat < hosts.txt | echo {0:0}"), "web-01");
    assert_eq!(shell("cat hosts.txt | echo {0:0} > out; cat out"), "web-01");
    assert_eq!(shell("cat nope 2>/dev/null | echo [{0:0}]"), "[]");
    // A stage that itself used a coordinate is just another stream.
    assert_eq!(
        shell(r#"printf "p q\n" | echo {0:1} | echo second=[{0:0}]"#),
        "second=[q]"
    );
}

/// Reaching back several stages, and every word of every line.
#[test]
fn reaching_back_and_reaching_wide() {
    assert_eq!(shell("echo w x y z | cat | cat | echo {0:2}"), "y");
    assert_eq!(shell("echo w x y z | cat | cat | echo {2:0:0}"), "w");
    assert_eq!(
        shell(r#"printf "a b\nc d\n" | printf "<%s>" {*:*}"#),
        "<a><b><c><d>"
    );
}

/// **Bytes that are not text do not bring the shell down.** A stream is whatever the command
/// printed, and some commands print anything at all.
#[test]
fn hostile_bytes_are_survivable() {
    // Invalid UTF-8 is replaced, not fatal.
    assert_eq!(shell(r#"printf "\xff\xfe ok\n" | echo [{0:1}]"#), "[ok]");
    // NULs are dropped, as command substitution drops them: a shell word is a C string.
    assert_eq!(shell(r#"printf "a\0b c\n" | echo [{0:1}]"#), "[c]");
    // A stage that dies part-way still leaves what it managed to print.
    assert_eq!(
        shell(r#"sh -c "echo one; kill -9 \$\$" | echo [{0:0}]"#),
        "[one]"
    );
    // A great many lines and a great many words.
    assert_eq!(shell("seq 1 20000 | echo [{-1:0}]"), "[20000]");
}

/// **A coordinate works everywhere a word can go**, not only in the argument list.
///
/// All three of these used to be left as text and then read as nothing: the command ran and did the
/// wrong thing without saying so. A redirection wrote to a file literally called `{0:0}`, an
/// assignment set the literal text, and a compound stage printed a blank.
#[test]
fn a_coordinate_works_wherever_a_word_does() {
    // A redirection target.
    assert_eq!(shell("cat hosts.txt | cat > {0:0}; ls web-01"), "web-01");
    // An assignment prefix.
    assert_eq!(
        shell(r#"cat hosts.txt | x={0:0} sh -c 'echo [$x]'"#),
        "[web-01]"
    );
    // A subshell and a group.
    assert_eq!(shell("cat hosts.txt | (echo [{0:0}])"), "[web-01]");
    assert_eq!(shell("cat hosts.txt | { echo [{0:0}]; }"), "[web-01]");
    // A condition and a case subject.
    assert_eq!(
        shell("cat hosts.txt | if true; then echo [{0:0}]; fi"),
        "[web-01]"
    );
    assert_eq!(
        shell("cat hosts.txt | case {0:0} in web-01) echo matched;; esac"),
        "matched"
    );
}

/// **The xargs shape, with no new keyword.** `for` plus `{*:n}` is the "run this once per line"
/// case, using the loop the shell already had — which is why this design has no `each` builtin.
#[test]
fn for_over_a_coordinate_is_the_iteration_case() {
    assert_eq!(
        shell("cat hosts.txt | for h in {*:0}; do echo host=$h; done"),
        "host=web-01\nhost=web-02\nhost=db-01"
    );
    assert_eq!(
        shell("cat hosts.txt | for a in {*:1}; do echo ip=$a; done"),
        "ip=10.0.0.1\nip=10.0.0.2\nip=10.0.0.9"
    );
}

/// **A function definition is not rewritten.** Its body runs later, when this stream is gone, so
/// baking today's text into it would make the definition mean something other than what was
/// written.
#[test]
fn a_function_body_is_not_baked() {
    assert_eq!(
        shell(r#"cat hosts.txt | { f(){ echo "body {0:0}"; }; f; }"#),
        "body {0:0}"
    );
}

/// **The gate must see everywhere the rewriter writes.** A gate that read only the argument list
/// would leave these on the concurrent path, where the rewriter never runs — so the substitution
/// would simply not happen, silently. Each of these is a place only the gate's walk can find.
#[test]
fn the_gate_sees_everywhere_the_rewriter_writes() {
    for line in [
        "cat hosts.txt | cat > {0:0}",
        "cat hosts.txt | x={0:0} true",
        "cat hosts.txt | (true {0:0})",
        "cat hosts.txt | for x in {0:0}; do true; done",
        "cat hosts.txt | case {0:0} in *) true;; esac",
    ] {
        // If the gate missed it the coordinate would survive into the output as literal text.
        let out = shell(&format!("{line}; echo done"));
        assert!(!out.contains("{0:0}"), "the gate missed {line:?}: {out:?}");
    }
}

/// **Where the recursion stops: a nested pipeline owns its own stream.**
///
/// `cat f | (echo {0:0})` names the outer pipe, because the subshell has nothing feeding it. But a
/// pipeline *inside* a loop body has its own upstream, and its coordinate means that one — so it
/// must not be rewritten from outside. Getting this wrong made the loop print blanks: the outer
/// walk claimed the inner coordinate and resolved it against a stream that did not exist.
#[test]
fn a_nested_pipeline_keeps_its_own_stream() {
    // The inner pipeline resolves against its own upstream, once per iteration.
    assert_eq!(
        shell(r#"for i in 1 2 3; do printf "v$i\n" | echo [{0:0}]; done"#),
        "[v1]\n[v2]\n[v3]"
    );
    // A loop *as a stage* still reads the outer stream, because it has no pipeline of its own.
    assert_eq!(
        shell("cat hosts.txt | for h in {*:0}; do echo $h; done"),
        "web-01\nweb-02\ndb-01"
    );
    // And both at once: the loop reads the outer stream, the inner pipeline reads its own.
    assert_eq!(
        shell(r#"cat hosts.txt | for h in {*:0}; do printf "x-$h\n" | echo [{0:0}]; done"#),
        "[x-web-01]\n[x-web-02]\n[x-db-01]"
    );
}

/// **`[[ … ]]` and `test` must agree**, and they did not.
///
/// `[[ ]]` wraps each operand in double quotes so it cannot split — which also hid the coordinate
/// from the substitution, so `test {0:0} = alpha` was true while `[[ {0:0} == alpha ]]` was false.
/// The same test written two ways disagreeing.
///
/// The fix has a precedent in the same function: `@name` is left unwrapped there for exactly this
/// reason. A substituted value arrives already quoted, so leaving it bare cannot split or glob.
#[test]
fn a_conditional_agrees_with_test() {
    assert_eq!(
        shell("cat hosts.txt | [[ {0:0} == web-01 ]] && echo yes"),
        "yes"
    );
    assert_eq!(
        shell("cat hosts.txt | test {0:0} = web-01 && echo yes"),
        "yes"
    );
    assert_eq!(
        shell("cat hosts.txt | [[ {0:0} != zzz ]] && echo yes"),
        "yes"
    );
    // And brace expansion inside `[[ ]]` is untouched by the change.
    assert_eq!(shell(r#"[[ "{0..2}" == "{0..2}" ]] && echo yes"#), "yes");
}

/// The compound branches the first pass did not reach.
#[test]
fn every_compound_branch_substitutes() {
    assert_eq!(
        shell("cat hosts.txt | if false; then echo a; elif true; then echo [{0:0}]; fi"),
        "[web-01]"
    );
    assert_eq!(
        shell("cat hosts.txt | if false; then echo a; else echo [{0:0}]; fi"),
        "[web-01]"
    );
    assert_eq!(
        shell("cat hosts.txt | for ((i=0;i<2;i++)); do echo [{0:0}]; done"),
        "[web-01]\n[web-01]"
    );
    assert_eq!(
        shell("cat hosts.txt | case zz in web-01) echo no;; *) echo [{0:0}];; esac"),
        "[web-01]"
    );
    // A coordinate as a case *pattern*.
    assert_eq!(
        shell("cat hosts.txt | case web-01 in {0:0}) echo matched;; esac"),
        "matched"
    );
}

/// A heredoc body and an array literal are words too.
#[test]
fn heredocs_and_arrays_substitute() {
    assert_eq!(
        shell("cat hosts.txt | cat <<EOF\nhere=[{0:0}]\nEOF"),
        "here=[web-01]"
    );
    assert_eq!(
        shell(r#"cat hosts.txt | { a=({0:0} {1:0}); echo "[${a[0]}][${a[1]}]"; }"#),
        "[web-01][web-02]"
    );
}

/// Traps, exit status and negation behave as they do anywhere else.
#[test]
fn the_surrounding_shell_is_unchanged() {
    assert_eq!(
        shell(r#"cat hosts.txt | { trap "echo trapped" EXIT; echo [{0:0}]; }"#),
        "[web-01]\ntrapped"
    );
    assert_eq!(
        shell("cat hosts.txt | { echo [{0:0}]; exit 7; }; echo rc=$?"),
        "[web-01]\nrc=7"
    );
    assert_eq!(shell("! cat hosts.txt | grep -q {0:0}; echo rc=$?"), "rc=1");
}

/// The walk recurses, so it must not run out of stack on a deeply nested stage.
#[test]
fn deep_nesting_is_survivable() {
    let depth = 80;
    let line = format!(
        "cat hosts.txt | {}echo [{{0:0}}]{}",
        "{ ".repeat(depth),
        "; }".repeat(depth)
    );
    assert_eq!(shell(&line), "[web-01]");
}
