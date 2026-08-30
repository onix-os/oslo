//! What a redirection on a structured stage means.
//!
//! Rows cross between structured stages **in memory**, not on a descriptor. So a redirection is not
//! the same question it is on the byte path, and answering it as though it were produced two bugs
//! that looked unrelated:
//!
//! * `… | first 2 2>/dev/null | cat` answered `lines: command not found`. Any redirection at all
//!   counted as "this stage's output went to a file", so a *stderr* one — which cannot touch a row —
//!   forced text on the stage, left no structured edge, and dropped the whole line onto the byte
//!   path where the verbs are not commands.
//! * `ls | first 2 > mid.txt | cat` answered `first: command not found` **and created an empty
//!   `mid.txt`**, because the byte path applies a redirection before finding there is nothing to run.

mod common;

/// **A stderr redirection does not change what the pipeline is.** The rows are untouched by it, so
/// the structured edge has to survive it.
#[test]
fn a_stderr_redirection_leaves_the_rows_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(
        dir.path(),
        "printf 'a\\nb\\nc\\n' | lines | first 2 2>/dev/null | cat",
    );
    assert_eq!(run.out(), "a\nb", "stderr: {}", run.stderr);
    assert_eq!(run.status, 0);
}

/// And it is *applied*, rather than merely tolerated: a verb that complains is quiet under it.
#[test]
fn a_stderr_redirection_is_actually_applied() {
    let dir = tempfile::tempdir().expect("tempdir");

    let loud = common::run_in(
        dir.path(),
        "printf 'a\\n' | lines | where 'nosuchfn()' | cat",
    );
    assert!(
        loud.stderr.contains("nil value"),
        "the failure is reported when nothing suppresses it: {}",
        loud.stderr
    );

    let quiet = common::run_in(
        dir.path(),
        "printf 'a\\n' | lines | where 'nosuchfn()' 2>/dev/null | cat",
    );
    assert!(
        !quiet.stderr.contains("nil value"),
        "the redirection did not reach the stage: {}",
        quiet.stderr
    );
}

/// **The refusal names the real problem and leaves no file behind.** Both halves matter: the old
/// diagnostic named a command that exists, and the empty file was a side effect of a pipeline that
/// never ran.
#[test]
fn a_middle_verb_that_redirects_its_output_is_refused_without_side_effects() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(
        dir.path(),
        "printf 'a\\nb\\nc\\n' | lines | first 2 > mid.txt | cat",
    );

    assert_eq!(run.status, 2, "a refusal, not a success: {}", run.stderr);
    assert!(
        run.stderr.contains("cannot redirect its output"),
        "the message names the real problem: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("command not found"),
        "and not a name that exists: {}",
        run.stderr
    );
    assert!(
        !dir.path().join("mid.txt").exists(),
        "a pipeline that did not run left a file behind"
    );
}

/// The last stage is the one whose redirection the structured half can apply, and it still does.
#[test]
fn the_last_stage_may_still_redirect() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(dir.path(), "printf 'a\\nb\\n' | lines | first 1 > out.txt");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let written = std::fs::read_to_string(dir.path().join("out.txt")).expect("out.txt");
    assert_eq!(written.trim(), "a");
}

/// **A pipeline with no structured verb in it is untouched**, which is the POSIX question: a middle
/// redirection is ordinary shell, and every script on the machine uses it.
#[test]
fn a_byte_pipeline_with_a_middle_redirection_is_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(dir.path(), "printf 'x\\n' | cat > mid.txt | cat");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let written = std::fs::read_to_string(dir.path().join("mid.txt")).expect("mid.txt");
    assert_eq!(
        written, "x\n",
        "the middle stage wrote its file, as in bash"
    );
}
