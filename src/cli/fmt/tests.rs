use super::*;

fn args(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

fn scratch(name: &str, contents: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("oslo-fmt-{}-{name}", std::process::id()));
    std::fs::write(&path, contents).expect("a writable temporary directory");
    path
}

#[test]
fn an_option_it_does_not_have_is_refused() {
    assert_eq!(run(&args("--nope")), 2);
    assert_eq!(run(&args("--indent wide x.sh")), 2);
    assert_eq!(run(&args("--indent")), 2);
}

/// There is nothing to write over and nothing to compare against.
#[test]
fn writing_and_checking_need_files() {
    assert_eq!(run(&args("-w")), 2);
    assert_eq!(run(&args("--check")), 2);
}

#[test]
fn help_is_an_answer_rather_than_a_mistake() {
    assert_eq!(run(&args("--help")), 0);
}

/// **A file that does not parse is left exactly as it was.** The whole argument for refusing: the
/// output would be a second mistake laid over the first.
#[test]
fn a_script_that_will_not_parse_is_reported_and_untouched() {
    let broken = "if a; then\n";
    let path = scratch("broken.sh", broken);
    assert_eq!(run(&[path.display().to_string()]), 2);
    assert_eq!(
        run(&["-w".to_string(), path.display().to_string()]),
        2,
        "-w must not write over it either"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    let _ = std::fs::remove_file(&path);
}

/// `--check` changes nothing and says so with its status.
#[test]
fn check_answers_one_without_touching_anything() {
    let untidy = "if a\nthen\nb\nfi\n";
    let path = scratch("untidy.sh", untidy);
    assert_eq!(run(&["--check".to_string(), path.display().to_string()]), 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), untidy);

    assert_eq!(run(&["-w".to_string(), path.display().to_string()]), 0);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "if a; then\n    b\nfi\n"
    );
    // Already laid out, so there is nothing to report the second time.
    assert_eq!(run(&["--check".to_string(), path.display().to_string()]), 0);
    let _ = std::fs::remove_file(&path);
}

/// A file that cannot be read is a failure, not silence.
#[test]
fn a_missing_file_is_reported() {
    assert_eq!(run(&["/nonexistent/nowhere.sh".to_string()]), 2);
}

/// Standard input is the default because it is what makes this usable in a pipe; naming it beside
/// files would be two sources for one output.
#[test]
fn standard_input_cannot_be_mixed_with_files() {
    assert_eq!(run(&args("- x.sh")), 2);
}
