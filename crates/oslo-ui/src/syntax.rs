//! Is the line the user just pressed Enter on finished?
//!
//! The prompt needs a three-way answer — finished, unfinished, wrong — and the only thing that
//! can give it honestly is the parser the shell actually runs. The old validator counted `'` and
//! `"` by hand: `echo don\'t` looked like an open quote and wedged the prompt until Ctrl-C, while
//! `for i in 1 2 3` and `cat <<EOF` looked finished and were handed straight to the executor,
//! the second one running `cat` with no here-document at all.
//!
//! `rune` answers all three directly: it distinguishes a construct that was still open when the
//! input ran out from text that is not shell at all, which is the whole reason this is a lookup
//! rather than a second parser.

/// What the accumulated prompt buffer amounts to so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputStatus {
    /// A complete program. Run it.
    Complete,
    /// Syntactically fine but unfinished; read another line.
    Incomplete,
    /// Not shell. Hand it over so the executor reports the error the same way a script would.
    Invalid,
}

/// The default PS2, used when the environment does not set one.
pub const DEFAULT_PS2: &str = "> ";

/// Classify a prompt buffer.
///
/// An empty or blank buffer is [`InputStatus::Complete`]: pressing Enter on nothing must give a
/// fresh prompt, not a continuation.
pub fn classify(input: &str) -> InputStatus {
    if input.trim().is_empty() {
        return InputStatus::Complete;
    }

    // Guard the same stack overflow `parse_bash_script` guards: a pasted line of ten thousand open
    // parentheses would abort the process from inside whatever walks the tree, where there is no
    // error path at all.
    if oslo_base::nesting::check_nesting(input).is_err() {
        return InputStatus::Invalid;
    }

    // The trailing newline matters: without it a here-document whose terminator is the last line
    // typed still looks unterminated, so `cat <<EOF … EOF` would never finish.
    let buf = format!("{input}\n");
    match rune::parse(&buf).completeness() {
        rune::Completeness::Complete => InputStatus::Complete,
        rune::Completeness::Unfinished => InputStatus::Incomplete,
        rune::Completeness::Invalid => InputStatus::Invalid,
    }
}

/// Whether `line` opens a here-document, so that what follows it is a body rather than a command.
///
/// [`classify`] already knows this — it is why `cat <<EOF` comes back [`InputStatus::Incomplete`]
/// — but it cannot *say* it: "the input ended in the middle of something" is the same answer for
/// an open quote, and those are not the same thing at all. The REPL's history expansion has to
/// tell them apart, because a here-document body is **data**: a `!` in it must reach the file
/// being written, not be replaced by a previous command (`startup::repl::read_command`).
///
/// A scanner rather than a parse, deliberately — it must answer for a line the parser cannot finish
/// yet, which is every line that opens a here-document.
pub fn opens_here_document(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut quote = super::words::Quote::None;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match quote {
            super::words::Quote::Single => {
                if c == '\'' {
                    quote = super::words::Quote::None;
                }
            }
            super::words::Quote::Double => {
                if c == '\\' {
                    i += 1;
                } else if c == '"' {
                    quote = super::words::Quote::None;
                }
            }
            super::words::Quote::None => match c {
                '\\' => i += 1,
                '\'' => quote = super::words::Quote::Single,
                '"' => quote = super::words::Quote::Double,
                '<' if bytes.get(i + 1) == Some(&b'<') => {
                    // `<<<` is a here-string: it takes its body from the same line.
                    if bytes.get(i + 2) != Some(&b'<') {
                        return true;
                    }
                    i += 2;
                }
                _ => {}
            },
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_lines_are_complete() {
        for line in [
            "echo hi",
            "",
            "   ",
            "for i in 1 2 3; do echo $i; done",
            "cat <<EOF\nbody\nEOF",
            "echo 'a b'",
            "echo \"a b\"",
            "if true; then echo y; fi",
            "echo hi # done",
        ] {
            assert_eq!(classify(line), InputStatus::Complete, "{line:?}");
        }
    }

    #[test]
    fn escaped_quote_does_not_wedge_the_prompt() {
        // The reproduction from PLAN R9.1: the old validator counted the escaped quote and asked
        // for a continuation line forever.
        assert_eq!(classify(r"echo don\'t"), InputStatus::Complete);
        assert_eq!(classify(r#"echo "don\"t""#), InputStatus::Complete);
    }

    #[test]
    fn unfinished_compound_commands_continue() {
        for line in [
            "for i in 1 2 3",
            "for i in 1 2 3; do",
            "if true",
            "if true; then",
            "while read l; do",
            "case $x in",
            "f() {",
            "echo a &&",
            "echo a |",
        ] {
            assert_eq!(classify(line), InputStatus::Incomplete, "{line:?}");
        }
    }

    #[test]
    fn unterminated_quotes_continue() {
        assert_eq!(classify("echo 'a"), InputStatus::Incomplete);
        assert_eq!(classify("echo \"a"), InputStatus::Incomplete);
        assert_eq!(classify("echo $(ls"), InputStatus::Incomplete);
        assert_eq!(classify("echo `ls"), InputStatus::Incomplete);
    }

    #[test]
    fn an_open_here_document_continues() {
        // The worst of the three R9.1 bugs: this used to be accepted and run with an empty body.
        assert_eq!(classify("cat <<EOF"), InputStatus::Incomplete);
        assert_eq!(classify("cat <<EOF\nbody"), InputStatus::Incomplete);
        assert_eq!(classify("cat <<-EOF\n\tbody"), InputStatus::Incomplete);
    }

    #[test]
    fn genuine_syntax_errors_are_not_continuations() {
        for line in ["done", "fi", "echo hi )", "esac"] {
            assert_eq!(classify(line), InputStatus::Invalid, "{line:?}");
        }
    }

    #[test]
    fn here_document_introducers_are_detected() {
        assert!(opens_here_document("cat <<EOF"));
        assert!(opens_here_document("cat <<-EOF"));
        assert!(opens_here_document("cat <<'EOF'"));
        assert!(opens_here_document("sort <<EOF | uniq"));
        assert!(!opens_here_document("cat <<<word"));
        assert!(!opens_here_document("echo '<<EOF'"));
        assert!(!opens_here_document("echo a < b"));
        assert!(!opens_here_document(r"echo \<\<EOF"));
    }

    /// The distinction the REPL's history expansion depends on: a line that is unfinished because
    /// a quote is open is not a line that opens a here-document, even though [`classify`] gives
    /// both the same answer.
    #[test]
    fn an_open_quote_is_not_a_here_document() {
        assert_eq!(classify("echo 'a"), InputStatus::Incomplete);
        assert!(!opens_here_document("echo 'a"));
        assert_eq!(classify("cat <<EOF"), InputStatus::Incomplete);
        assert!(opens_here_document("cat <<EOF"));
    }

    #[test]
    fn absurd_nesting_is_rejected_rather_than_crashing() {
        let deep = "(".repeat(100_000);
        assert_eq!(classify(&deep), InputStatus::Invalid);
    }
}
