//! Which colour each span of a line ends up in.
//!
//! The lexer half is tested next door; these are the cases that need a shell to answer —
//! whether a name is a builtin, a function, or nothing at all.

use super::*;

/// A context that answers from fixed sets, so classification is testable with no shell.
fn ctx<'a>(
    builtins: &'a dyn Fn(&str) -> bool,
    functions: &'a dyn Fn(&str) -> bool,
    check_paths: bool,
) -> Context<'a> {
    Context {
        path: "/nonexistent-zz",
        is_builtin: builtins,
        is_function: functions,
        check_paths,
    }
}

fn kinds(line: &str, ctx: &Context<'_>) -> Vec<(String, TokenType)> {
    classify(&lex(line), ctx)
        .into_iter()
        .filter(|(_, t)| *t != TokenType::Plain)
        .collect()
}

/// **The escaped forms are resolved by what they will actually reach.**
///
/// Looked up with the backslash still attached they matched nothing and painted red, which is
/// how a line that runs perfectly came to read as a mistake — the same bug `=cmd` had.
/// `$PATH` here is a directory that does not exist, so nothing resolves as a program and the
/// difference between the two forms is the whole of what this checks.
#[test]
fn an_escaped_command_is_resolved_without_its_backslash() {
    let builtins = |n: &str| n == "rm";
    let functions = |n: &str| n == "rm";
    let c = ctx(&builtins, &functions, false);

    // Plain `rm` is the shell's own.
    assert_eq!(kinds("rm x", &c)[0].1, TokenType::Builtin);
    // `\rm` skips the builtin *and* the function, so only a program could answer — and on
    // this `$PATH` none does.
    assert_eq!(kinds(r"\rm x", &c)[0].1, TokenType::Error);
    // `\\rm` leaves the function in the running, so it resolves to that.
    assert_eq!(kinds(r"\\rm x", &c)[0].1, TokenType::Function);
    // The span still carries its backslashes: the colour changed, not the text.
    assert_eq!(kinds(r"\\rm x", &c)[0].0, r"\\rm");
}

/// `\sudo` still runs the rest of the line as somebody else.
#[test]
fn an_escaped_command_keeps_the_danger_colour() {
    let none = |_: &str| false;
    let c = ctx(&none, &none, false);
    assert_eq!(kinds(r"\sudo rm -rf /", &c)[0].1, TokenType::Danger);
}

#[test]
fn a_command_resolves_to_one_of_four_colours() {
    let builtins = |n: &str| n == "cd";
    let functions = |n: &str| n == "deploy";
    let c = ctx(&builtins, &functions, false);

    assert_eq!(kinds("cd /tmp", &c)[0].1, TokenType::Builtin);
    assert_eq!(kinds("deploy now", &c)[0].1, TokenType::Function);
    // Nothing resolves it, so it is wrong — fish's most useful colour.
    assert_eq!(kinds("nosuchcmd-zz x", &c)[0].1, TokenType::Error);
    assert_eq!(kinds("if true; then fi", &c)[0].1, TokenType::Keyword);
}

/// An absolute path that is not there is as wrong as a name that is not there.
#[test]
fn a_path_command_is_checked_as_a_path() {
    let no = |_: &str| false;
    let c = ctx(&no, &no, false);
    assert_eq!(kinds("/nonexistent-zz/prog x", &c)[0].1, TokenType::Error);
    assert_eq!(kinds("/bin/sh -c x", &c)[0].1, TokenType::Command);
}

#[test]
fn options_and_parameters_are_told_apart() {
    let no = |_: &str| false;
    let c = ctx(&no, &no, false);
    let seen = kinds("cmd -l --long plain", &c);
    assert_eq!(seen[1].1, TokenType::Option);
    assert_eq!(seen[2].1, TokenType::Option);
    assert_eq!(seen[3].1, TokenType::Param);
    // A bare `-` is a parameter, not an option: it means stdin to half the tools there are.
    assert_eq!(kinds("cmd -", &c)[1].1, TokenType::Param);
}

/// The colour that tells you the file you named is really there.
#[test]
fn a_parameter_naming_a_real_file_is_marked() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("here.txt");
    std::fs::write(&file, b"x").expect("write");
    let no = |_: &str| false;
    let c = ctx(&no, &no, true);

    let line = format!(
        "cmd {} {}",
        file.display(),
        dir.path().join("gone").display()
    );
    let seen = kinds(&line, &c);
    assert_eq!(seen[1].1, TokenType::ValidPath);
    assert_eq!(seen[2].1, TokenType::Param);
}

/// A syscall per word per keystroke is exactly what the command index exists to avoid, so the
/// number of them is bounded rather than left to the length of the line.
#[test]
fn path_checking_is_capped_and_can_be_turned_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut names = Vec::new();
    for i in 0..12 {
        let f = dir.path().join(format!("f{i}"));
        std::fs::write(&f, b"x").expect("write");
        names.push(f.display().to_string());
    }
    let no = |_: &str| false;

    let line = format!("cmd {}", names.join(" "));
    let capped = kinds(&line, &ctx(&no, &no, true));
    let marked = capped
        .iter()
        .filter(|(_, t)| *t == TokenType::ValidPath)
        .count();
    assert!(
        marked <= MAX_PATH_CHECKS,
        "{marked} paths checked, cap is {MAX_PATH_CHECKS}"
    );

    // And off entirely, nothing is stat'd.
    let off = kinds(&line, &ctx(&no, &no, false));
    assert!(off.iter().all(|(_, t)| *t != TokenType::ValidPath));
}

#[test]
fn every_lexical_role_reaches_a_colour() {
    let no = |_: &str| false;
    let c = ctx(&no, &no, false);
    let seen = kinds(r#"echo "q" $V >f 2>&1 | wc; true & # note"#, &c);
    let types: Vec<TokenType> = seen.iter().map(|(_, t)| *t).collect();
    for wanted in [
        TokenType::DoubleQuote,
        TokenType::Variable,
        TokenType::Redirection,
        TokenType::Operator,
        TokenType::End,
        TokenType::Comment,
    ] {
        assert!(types.contains(&wanted), "{wanted:?} missing from {seen:?}");
    }
}

#[test]
fn painting_reassembles_the_line_once_the_escapes_are_stripped() {
    let _held = theme::held_at(theme::Depth::Ansi16);
    let no = |_: &str| false;
    let line = "echo 'a b' $HOME | wc -l";
    let painted = paint(line, &ctx(&no, &no, false));
    let stripped: String = {
        let mut out = String::new();
        let mut chars = painted.chars();
        while let Some(ch) = chars.next() {
            if ch != '\x1b' {
                out.push(ch);
                continue;
            }
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
        out
    };
    assert_eq!(stripped, line);
}

/// Strip the escapes from a painted line, leaving what the terminal actually shows.
fn shown(painted: &str) -> String {
    let mut out = String::new();
    let mut chars = painted.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}

/// Danger padding preserves the source line's cell count.
#[test]
fn padding_sudo_does_not_change_a_single_column() {
    let _held = theme::held_at(theme::Depth::Ansi16);
    let no = |_: &str| false;
    for line in [
        "sudo ls",
        "sudo",
        "echo a && sudo ls -l",
        "sudo  ls",
        "  sudo ls",
    ] {
        assert_eq!(
            shown(&paint(line, &ctx(&no, &no, false))),
            line,
            "the painted line must be the line"
        );
    }
}

/// And it really does widen the field, or the test above would pass on a no-op.
#[test]
fn the_red_field_reaches_past_the_word() {
    let _held = theme::held_at(theme::Depth::Ansi16);
    let no = |_: &str| false;
    let mut tokens = classify(&lex("echo a && sudo ls"), &ctx(&no, &no, false));
    pad_danger(&mut tokens);
    let danger: Vec<&String> = tokens
        .iter()
        .filter(|(_, kind)| *kind == TokenType::Danger)
        .map(|(text, _)| text)
        .collect();
    assert_eq!(danger, vec![" sudo "], "a space of red on each side");
}

/// A builtin is answered by the shell, so it resolves on a `$PATH` that does not exist — and a
/// name nothing answers for is an error on the same `$PATH`, which is what says the first result
/// came from the builtin and not from a lookup that happened to succeed.
#[test]
fn a_builtin_resolves_without_touching_the_disk() {
    let builtins = |n: &str| n == "cd";
    let none = |_: &str| false;
    let c = ctx(&builtins, &none, false);

    assert_eq!(kinds("cd /tmp", &c)[0].1, TokenType::Builtin);
    assert_eq!(kinds("definitely-not-a-command", &c)[0].1, TokenType::Error);
}
