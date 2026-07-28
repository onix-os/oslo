//! A nesting-depth pre-check on raw shell source, run before the parser sees it.
//!
//! `brush_parser` is recursive descent, and rush's AST conversion, evaluator and even the `Drop`
//! glue that frees the AST all recurse over the same shape. Deeply nested input therefore
//! overflows the stack *inside brush*, before any rush code that could report the problem runs —
//! and Rust turns a stack overflow into `SIGABRT`, so the shell died with status 134 and a core
//! dump. Nothing downstream can defend against that: the only place to stop it is before the
//! parser is called at all.
//!
//! The scan is deliberately approximate, and does not pretend to be a lexer. It tracks quoting
//! and comments well enough not to count the parenthesis in `echo "smile :("`, pairs each closer
//! against the opener it actually closes so a stray `)` cannot cancel an unrelated `(`, and
//! refuses only input nested far beyond anything a person writes. Being slightly wrong about
//! depth 100 costs nothing; having no check at all cost the process.

use crate::error::{Result, ShellError};

/// Deepest nesting the parser is allowed to see.
///
/// Measured, not guessed: a debug build overflows its 8 MiB stack somewhere between 400 and 600
/// levels of `{ …; }`, and a nested program burns that stack at the same time as the function-call
/// and `source` chains bounded in [`crate::env::nesting`], so all three limits share one budget.
/// Real scripts do not come close: the deepest nesting in a large shell codebase is single digits.
pub const MAX_INPUT_NESTING: usize = 100;

/// Refuse input whose nesting would overflow the stack inside the parser.
pub fn check_nesting(input: &str) -> Result<()> {
    if max_nesting_depth(input) > MAX_INPUT_NESTING {
        return Err(ShellError::ExecutionError(
            "maximum nesting level exceeded".to_string(),
        ));
    }
    Ok(())
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Quote {
    None,
    Single,
    Double,
}

/// What is currently open, in the order it was opened.
///
/// Keeping the opener rather than a bare counter is what makes a mismatched closer harmless: the
/// `)` that ends a `case` pattern, or a lone `)` inside a heredoc body, finds a `case` or a `{` on
/// top and is ignored instead of cancelling a level that is genuinely open.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Open {
    Paren,
    Brace,
    If,
    Case,
    Do,
}

fn open(stack: &mut Vec<Open>, what: Open, max: &mut usize) {
    stack.push(what);
    *max = (*max).max(stack.len());
}

/// Close `what` if it is what is actually open; a closer that matches nothing is ignored.
fn close(stack: &mut Vec<Open>, what: Open) {
    if stack.last() == Some(&what) {
        stack.pop();
    }
}

/// The deepest simultaneous nesting anywhere in `input`.
fn max_nesting_depth(input: &str) -> usize {
    let mut stack: Vec<Open> = Vec::new();
    let mut max = 0usize;
    let mut quote = Quote::None;
    let mut escaped = false;
    let mut comment = false;
    let mut word = String::new();
    // Reserved words nest only where a command may start: `grep if file` must not count.
    let mut cmd_pos = true;
    let mut prev_dollar = false;

    for c in input.chars() {
        if escaped {
            escaped = false;
            prev_dollar = false;
            continue;
        }
        if comment {
            if c == '\n' {
                comment = false;
                cmd_pos = true;
            }
            continue;
        }

        match quote {
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                }
                continue;
            }
            Quote::Double => {
                // Only a substitution nests inside a double-quoted string: `"$(cmd)"` and `"${v}"`
                // send the parser down a level, a smiley does not.
                match c {
                    '\\' => escaped = true,
                    '"' => quote = Quote::None,
                    '(' if prev_dollar => open(&mut stack, Open::Paren, &mut max),
                    '{' if prev_dollar => open(&mut stack, Open::Brace, &mut max),
                    ')' => close(&mut stack, Open::Paren),
                    '}' => close(&mut stack, Open::Brace),
                    _ => {}
                }
                prev_dollar = c == '$';
                continue;
            }
            Quote::None => {}
        }

        prev_dollar = c == '$';

        match c {
            '\\' => {
                escaped = true;
                word.clear();
                cmd_pos = false;
            }
            '\'' => {
                quote = Quote::Single;
                word.clear();
                cmd_pos = false;
            }
            '"' => {
                quote = Quote::Double;
                word.clear();
                cmd_pos = false;
            }
            // A `#` only starts a comment at the beginning of a word.
            '#' if word.is_empty() => comment = true,
            '(' | '{' => {
                open(
                    &mut stack,
                    if c == '(' { Open::Paren } else { Open::Brace },
                    &mut max,
                );
                word.clear();
                cmd_pos = true;
            }
            // A command may follow a closer directly — that is exactly what a `case` arm is —
            // so the next word is still a candidate keyword.
            ')' | '}' => {
                close(&mut stack, if c == ')' { Open::Paren } else { Open::Brace });
                word.clear();
                cmd_pos = true;
            }
            _ if c.is_ascii_alphanumeric() || c == '_' => word.push(c),
            // Any other character ends the current word, so decide what that word was.
            _ => {
                if cmd_pos {
                    match word.as_str() {
                        "if" => open(&mut stack, Open::If, &mut max),
                        "case" => open(&mut stack, Open::Case, &mut max),
                        // One level per loop body, whatever the loop keyword was.
                        "do" => open(&mut stack, Open::Do, &mut max),
                        "fi" => close(&mut stack, Open::If),
                        "esac" => close(&mut stack, Open::Case),
                        "done" => close(&mut stack, Open::Do),
                        _ => {}
                    }
                }
                cmd_pos = next_is_command_position(c, &word, cmd_pos);
                word.clear();
            }
        }
    }

    max
}

/// Whether the word after separator `c` (which followed `word`) may start a command.
fn next_is_command_position(c: char, word: &str, was_cmd_pos: bool) -> bool {
    match c {
        ';' | '&' | '|' | '\n' => true,
        // Indentation before a word does not move the position along.
        _ if word.is_empty() => was_cmd_pos,
        // These keywords introduce a command rather than being one.
        _ if c.is_whitespace() => matches!(word, "then" | "else" | "elif" | "do" | "in"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_INPUT_NESTING, check_nesting, max_nesting_depth};

    #[test]
    fn ordinary_scripts_are_shallow() {
        assert_eq!(max_nesting_depth("echo hello"), 0);
        assert_eq!(max_nesting_depth("f() { echo hi; }"), 1);
        assert_eq!(max_nesting_depth("if true; then echo hi; fi"), 1);
        assert_eq!(max_nesting_depth("for i in 1 2; do echo $i; done"), 1);
        assert_eq!(max_nesting_depth("echo $(echo ${x})"), 2);
        // `do`, `if`, and the substitution inside the body.
        assert_eq!(
            max_nesting_depth("while read l; do if [ -n \"$l\" ]; then echo \"$(date)\"; fi; done"),
            3
        );
    }

    /// A `case` arm's `)` has no matching `(`; before the opener stack it cancelled the `case`.
    #[test]
    fn case_arms_do_not_unbalance_the_count() {
        assert_eq!(
            max_nesting_depth("case $x in a) echo a;; *) echo b;; esac"),
            1
        );
        assert_eq!(
            max_nesting_depth("case $x in a) case $y in b) echo ab;; esac;; esac"),
            2
        );
    }

    /// The whole point of tracking quotes: text is not code.
    #[test]
    fn brackets_in_literal_text_do_not_count() {
        assert_eq!(max_nesting_depth("echo \"smile :(\""), 0);
        assert_eq!(max_nesting_depth("echo 'if ( { do'"), 0);
        assert_eq!(max_nesting_depth("echo \\( \\{"), 0);
        assert_eq!(max_nesting_depth("# if ( { do\necho hi"), 0);
        assert_eq!(max_nesting_depth("grep if file; echo do"), 0);
    }

    #[test]
    fn substitutions_inside_double_quotes_still_count() {
        assert_eq!(max_nesting_depth("echo \"$(echo \"$(date)\")\""), 2);
        assert_eq!(max_nesting_depth("echo \"${x}\""), 1);
    }

    #[test]
    fn deep_input_is_refused() {
        let n = MAX_INPUT_NESTING + 5;
        for (opener, closer) in [
            ("{ ", "; }"),
            ("( ", " )"),
            ("if true; then ", "; fi"),
            ("while true; do ", "; done"),
        ] {
            let script = format!("{}true{}", opener.repeat(n), closer.repeat(n));
            let err = check_nesting(&script).expect_err("must be refused");
            assert!(
                err.to_string().contains("maximum nesting level exceeded"),
                "{err}"
            );
        }
    }

    #[test]
    fn input_at_the_limit_is_accepted() {
        let n = MAX_INPUT_NESTING;
        let script = format!("{}true{}", "{ ".repeat(n), "; }".repeat(n));
        assert!(check_nesting(&script).is_ok());
    }
}
