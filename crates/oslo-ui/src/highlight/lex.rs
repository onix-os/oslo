//! Splitting a typed line into spans, with no environment and no disk.
//!
//! Purely lexical on purpose. Whether `foo` is a builtin, a function or nothing at all cannot be
//! answered here — it needs the shell — so this emits [`Span::CommandWord`] and lets
//! [`super::classify`] resolve it. The split is what makes the interesting half testable without
//! a `$PATH`, a filesystem or a terminal.

/// A lexical span. The concatenation of every `text` is the original line, byte for byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub role: Role,
}

/// What the lexer could work out on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A word in command position. Still to be resolved into builtin/function/command/error.
    CommandWord,
    /// A reserved word: `if`, `for`, `done`.
    Keyword,
    /// Any other word. Still to be resolved into option/valid path/plain parameter.
    Word,
    /// `'…'`, quotes included. Literal throughout, so it is never split.
    SingleQuote,
    /// The literal parts of a `"…"` string. What expands inside it is lit separately.
    DoubleQuote,
    /// A `\x` escape.
    Escape,
    /// A glob metacharacter: `*`, `?`, `[…]`. Lit apart from the word it sits in, because whether
    /// a word will expand is the thing you most want to know before pressing Enter.
    Glob,
    /// A bare number: `2` in `sleep 2`, `644` in `chmod 644`.
    Number,
    /// The `NAME=` of an assignment, without the value.
    Assignment,
    /// `$name`, `${name}`, `$(…)`.
    Variable,
    /// `>`, `>>`, `2>`, `<`, `<<`, `<<<`, `&>`.
    Redirection,
    /// `|`, `||`, `&&`.
    Operator,
    /// `;` and `&` — what fish calls `end`.
    End,
    /// `# …` to end of line.
    Comment,
    /// Whitespace, and anything with no colour of its own.
    Plain,
}

/// The reserved words a shell line can start a command with.
const KEYWORDS: [&str; 17] = [
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "function", "select", "time", "coproc",
];

/// Split `line` into spans.
pub fn lex(line: &str) -> Vec<Span> {
    let bytes = line.as_bytes();
    let mut spans: Vec<Span> = Vec::new();
    let mut i = 0;
    // Whether the next word is in command position. True at the start and after anything that
    // ends a command — which is why `;` and `|` reset it but a redirection does not.
    let mut command_position = true;

    while i < bytes.len() {
        let rest = &line[i..];
        let ch = rest.chars().next().expect("non-empty");

        if ch.is_whitespace() {
            let end = rest
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(rest.len());
            push(&mut spans, &rest[..end], Role::Plain);
            i += end;
            continue;
        }

        // A `#` is only a comment at the start of a word. `foo#bar` is one word and `$#` is a
        // parameter — the same rule the shell's own lexer follows, and getting it wrong here
        // would grey out the rest of a perfectly good line.
        if ch == '#' && starts_word(&spans) {
            push(&mut spans, rest, Role::Comment);
            break;
        }

        if ch == '\\' {
            // The escape and the character it escapes are one span, so a `\` at end of line does
            // not swallow the newline that is not there yet.
            let len = rest
                .chars()
                .take(2)
                .map(char::len_utf8)
                .sum::<usize>()
                .max(1);
            push(&mut spans, &rest[..len], Role::Escape);
            i += len;
            command_position = false;
            continue;
        }

        if ch == '\'' {
            // Single quotes take nothing back: everything inside is literal, so it is one span.
            let len = quoted_len(rest, ch);
            push(&mut spans, &rest[..len], Role::SingleQuote);
            i += len;
            command_position = false;
            continue;
        }

        if ch == '"' {
            // A double-quoted string still expands, so the `$var` inside it is lit as a variable
            // rather than swallowed by the string's colour. This is the single biggest difference
            // between a shell prompt that looks syntax-aware and one that looks like a text box —
            // `"$HOME/bin"` is two different things and should read as two.
            let len = quoted_len(rest, ch);
            push_double_quoted(&mut spans, &rest[..len]);
            i += len;
            command_position = false;
            continue;
        }

        if ch == '$' {
            let len = variable_len(rest);
            push(&mut spans, &rest[..len], Role::Variable);
            i += len;
            command_position = false;
            continue;
        }

        if let Some(len) = redirection_len(rest) {
            push(&mut spans, &rest[..len], Role::Redirection);
            i += len;
            // A redirection does not start a new command: `> out cmd` is not a thing, and the
            // word after it is the *target*, not a command name.
            command_position = false;
            continue;
        }

        if ch == '|' || ch == '&' || ch == ';' {
            let two = rest.starts_with("||") || rest.starts_with("&&") || rest.starts_with(";;");
            let len = if two { 2 } else { 1 };
            // `;` and a lone `&` separate commands; `|`, `||` and `&&` join them. Both start a
            // new command, but fish colours them differently and so does this.
            let role = if ch == ';' || (ch == '&' && !two) {
                Role::End
            } else {
                Role::Operator
            };
            push(&mut spans, &rest[..len], role);
            i += len;
            command_position = true;
            continue;
        }

        // An ordinary word, up to the next thing that is not part of one.
        let end = rest
            .find(|c: char| {
                c.is_whitespace()
                    || matches!(c, '|' | '&' | ';' | '<' | '>' | '\'' | '"' | '$' | '\\')
            })
            .unwrap_or(rest.len());
        let word = &rest[..end];

        // An assignment prefix — `FOO=bar cmd` — leaves the next word still in command position.
        let assignment = command_position && is_assignment(word);
        let role = if KEYWORDS.contains(&word) && command_position {
            Role::Keyword
        } else if command_position && !assignment {
            Role::CommandWord
        } else {
            Role::Word
        };
        if role == Role::Word {
            // An ordinary word is broken down further: a glob metacharacter, a bare number and
            // the `NAME=` of an assignment each get their own colour, so `chmod 644 *.rs` reads
            // as three different kinds of thing rather than one grey run.
            push_word(&mut spans, word);
        } else {
            push(&mut spans, word, role);
        }
        i += end;
        // A keyword does not consume the command position: `if grep …` still has `grep` as the
        // command. Nor does an assignment prefix.
        command_position = matches!(role, Role::Keyword) || assignment;
    }

    spans
}

/// Whether what comes next begins a word, which is what makes a `#` a comment.
fn starts_word(spans: &[Span]) -> bool {
    match spans.last() {
        None => true,
        Some(span) => matches!(
            span.role,
            Role::Plain | Role::Operator | Role::End | Role::Redirection
        ),
    }
}

/// Split an ordinary word into the parts worth colouring differently.
///
/// Globs first, because whether a word is going to *expand* is the thing you most want to know
/// before pressing Enter — `rm *.rs` and `rm '*.rs'` differ by one character and by everything.
fn push_word(spans: &mut Vec<Span>, word: &str) {
    if let Some(eq) = assignment_split(word) {
        push(spans, &word[..eq + 1], Role::Assignment);
        if eq + 1 < word.len() {
            push_word(spans, &word[eq + 1..]);
        }
        return;
    }
    if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
        push(spans, word, Role::Number);
        return;
    }
    let mut at = 0;
    while at < word.len() {
        let next = word[at..]
            .find(['*', '?', '[', ']'])
            .map(|n| at + n)
            .unwrap_or(word.len());
        if next > at {
            push(spans, &word[at..next], Role::Word);
        }
        if next < word.len() {
            let len = word[next..].chars().next().map(char::len_utf8).unwrap_or(1);
            push(spans, &word[next..next + len], Role::Glob);
            at = next + len;
        } else {
            at = next;
        }
    }
}

/// Where the `=` of a `NAME=value` word is, if it is one.
///
/// A *word*, not a command-position assignment: `--opt=value` and `FOO=bar` both read better with
/// the name apart from the value, and the shell's own rule about which is an assignment is about
/// where the word sits rather than how it looks.
fn assignment_split(word: &str) -> Option<usize> {
    let eq = word.find('=')?;
    if eq == 0 {
        return None;
    }
    let name = &word[..eq];
    name.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        .then_some(eq)
}

/// Split a double-quoted string into its literal parts and the expansions inside it.
fn push_double_quoted(spans: &mut Vec<Span>, text: &str) {
    let mut at = 0;
    let bytes = text.as_bytes();
    while at < text.len() {
        if bytes[at] == b'\\' {
            // An escape inside a double quote hides the next character from expansion.
            let len = text[at..]
                .chars()
                .take(2)
                .map(char::len_utf8)
                .sum::<usize>()
                .max(1);
            push(spans, &text[at..at + len], Role::Escape);
            at += len;
            continue;
        }
        if bytes[at] == b'$' {
            let len = variable_len(&text[at..]);
            if len > 1 {
                push(spans, &text[at..at + len], Role::Variable);
                at += len;
                continue;
            }
        }
        // Everything up to the next thing worth colouring, as one span.
        let next = text[at + 1..]
            .find(['$', '\\'])
            .map(|n| at + 1 + n)
            .unwrap_or(text.len());
        push(spans, &text[at..next], Role::DoubleQuote);
        at = next;
    }
}

fn push(spans: &mut Vec<Span>, text: &str, role: Role) {
    if !text.is_empty() {
        spans.push(Span {
            text: text.to_string(),
            role,
        });
    }
}

/// `NAME=` at the start of a word, which is a variable assignment rather than a command.
///
/// `pub(crate)` because the correction needs the same answer: the command word of `FOO=1 lsvlk` is
/// `lsvlk`, and repair was reading `FOO=1` and giving up — so the highlighter called the line a
/// command-not-found while the correction had nothing to say about it. Two rules would drift.
pub(crate) fn is_assignment(word: &str) -> bool {
    let Some(eq) = word.find('=') else {
        return false;
    };
    let name = &word[..eq];
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '[' || c == ']')
}

/// Length of a quoted run including both quotes, or to end of line if it never closes.
///
/// An unclosed quote is the normal state of a line being typed, so it is a span rather than an
/// error — the validator is what decides the line is unfinished.
fn quoted_len(rest: &str, quote: char) -> usize {
    let mut len = quote.len_utf8();
    let mut chars = rest[len..].chars();
    while let Some(c) = chars.next() {
        len += c.len_utf8();
        if c == quote {
            return len;
        }
        // Inside double quotes a backslash escapes the next character, so `"a\"b"` is one span.
        if c == '\\'
            && quote == '"'
            && let Some(next) = chars.next()
        {
            len += next.len_utf8();
        }
    }
    rest.len()
}

/// Length of a `$…` expansion.
fn variable_len(rest: &str) -> usize {
    let after = &rest[1..];
    match after.chars().next() {
        // `${…}` and `$(…)` run to their closing bracket, nested ones included.
        Some('{') => 1 + balanced_len(after, '{', '}'),
        Some('(') => 1 + balanced_len(after, '(', ')'),
        // The special parameters are one character each.
        Some(c) if "?!#*@$-0".contains(c) => 1 + c.len_utf8(),
        _ => {
            let end = after
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            // A bare `$` is just a dollar sign.
            1 + end
        }
    }
}

/// Length from `open` to its matching `close`, or to end of input.
fn balanced_len(text: &str, open: char, close: char) -> usize {
    let mut depth = 0usize;
    let mut len = 0usize;
    for c in text.chars() {
        len += c.len_utf8();
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return len;
            }
        }
    }
    text.len()
}

/// Length of a redirection operator at the start of `rest`, if there is one.
fn redirection_len(rest: &str) -> Option<usize> {
    // A leading file descriptor: `2>`, `10>&1`.
    let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(0);
    let after = &rest[digits..];

    // Longest first, or `<<` would be read as `<` and `<<<` as `<<`.
    const THREE: [&str; 1] = ["<<<"];
    const TWO: [&str; 6] = ["<<", ">>", "<>", ">&", "<&", "&>"];
    let operator = if THREE.iter().any(|op| after.starts_with(op)) {
        3
    } else if TWO.iter().any(|op| after.starts_with(op)) {
        2
    } else if after.starts_with('>') || after.starts_with('<') {
        1
    } else {
        return None;
    };

    let mut len = digits + operator;
    // `>&1` and `2>&1` take the descriptor that follows as part of the operator.
    if after[..operator].ends_with('&') {
        len += rest[len..].find(|c: char| !c.is_ascii_digit()).unwrap_or(0);
    }
    Some(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(line: &str) -> Vec<(String, Role)> {
        lex(line)
            .into_iter()
            .filter(|s| s.role != Role::Plain)
            .map(|s| (s.text, s.role))
            .collect()
    }

    /// The one invariant everything else rests on: the spans put the line back together exactly.
    #[test]
    fn spans_reassemble_into_the_original_line() {
        for line in [
            "echo hi",
            "ls -l | wc -l",
            "echo \"a b\" $HOME",
            "git commit -m 'x' && true",
            "cat <<EOF",
            "grep -n 'x' f.txt 2>&1 >out # a comment",
            "FOO=bar cmd arg",
            "echo ${HOME}/x $(date) \\$literal",
            "if true; then echo y; fi",
            "echo 'unclosed",
            "echo \\",
            "",
        ] {
            let joined: String = lex(line).iter().map(|s| s.text.as_str()).collect();
            assert_eq!(joined, line, "for {line:?}");
        }
    }

    #[test]
    fn the_first_word_of_each_command_is_a_command_word() {
        let found: Vec<String> = lex("ls | wc; grep x && true")
            .into_iter()
            .filter(|s| s.role == Role::CommandWord)
            .map(|s| s.text)
            .collect();
        assert_eq!(found, vec!["ls", "wc", "grep", "true"]);
    }

    /// A keyword does not consume the command position — `if grep` still has `grep` as the
    /// command, which is what makes an unknown command after `if` still show up as wrong.
    #[test]
    fn a_keyword_leaves_the_command_position_open() {
        let spans = lex("if grep x; then echo y; fi");
        let keywords: Vec<&str> = spans
            .iter()
            .filter(|s| s.role == Role::Keyword)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(keywords, vec!["if", "then", "fi"]);
        let commands: Vec<&str> = spans
            .iter()
            .filter(|s| s.role == Role::CommandWord)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(commands, vec!["grep", "echo"]);
    }

    /// `FOO=bar cmd` runs `cmd`, so the assignment must not eat the command position.
    #[test]
    fn an_assignment_prefix_is_not_the_command() {
        let spans = lex("FOO=bar BAZ=1 cmd arg");
        let commands: Vec<&str> = spans
            .iter()
            .filter(|s| s.role == Role::CommandWord)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(commands, vec!["cmd"]);
        // And `x=1` alone is not a command at all.
        assert!(
            lex("x=1").iter().all(|s| s.role != Role::CommandWord),
            "{:?}",
            lex("x=1")
        );
    }

    #[test]
    fn redirections_are_told_apart_from_operators_and_separators() {
        assert_eq!(
            roles("a > b >> c 2>&1 <<<x"),
            vec![
                ("a".into(), Role::CommandWord),
                (">".into(), Role::Redirection),
                ("b".into(), Role::Word),
                (">>".into(), Role::Redirection),
                ("c".into(), Role::Word),
                ("2>&1".into(), Role::Redirection),
                ("<<<".into(), Role::Redirection),
                ("x".into(), Role::Word),
            ]
        );
        // `;` and a lone `&` end a command; `|`, `||` and `&&` join two.
        assert_eq!(roles("a; b & c | d && e")[1].1, Role::End);
        assert_eq!(roles("a | b")[1].1, Role::Operator);
        assert_eq!(roles("a && b")[1].1, Role::Operator);
    }

    /// A `#` is a comment only at the start of a word. This is the same rule that had to be fixed
    /// in the shell's own lexer, and getting it wrong greys out the rest of a good line.
    #[test]
    fn a_hash_inside_a_word_is_not_a_comment() {
        assert!(roles("echo a#b").iter().all(|(_, r)| *r != Role::Comment));
        assert!(roles("echo $#").iter().all(|(_, r)| *r != Role::Comment));
        let commented = roles("echo x # note");
        assert_eq!(
            commented.last().map(|(t, r)| (t.as_str(), *r)),
            Some(("# note", Role::Comment))
        );
    }

    #[test]
    fn expansions_run_to_their_closing_bracket() {
        assert_eq!(roles("echo ${HOME}")[1], ("${HOME}".into(), Role::Variable));
        assert_eq!(roles("echo $(date)")[1], ("$(date)".into(), Role::Variable));
        // Nested, which a naive scan to the first `)` gets wrong.
        assert_eq!(
            roles("echo $(echo $(date))")[1],
            ("$(echo $(date))".into(), Role::Variable)
        );
        assert_eq!(roles("echo $?")[1], ("$?".into(), Role::Variable));
        assert_eq!(roles("echo $HOME")[1], ("$HOME".into(), Role::Variable));
    }

    /// A line being typed is usually unfinished, so an unclosed quote is a span rather than an
    /// error — deciding the line is incomplete is the validator's job, not the highlighter's.
    #[test]
    fn an_unclosed_quote_runs_to_the_end_rather_than_failing() {
        assert_eq!(roles("echo 'abc")[1], ("'abc".into(), Role::SingleQuote));
        assert_eq!(roles("echo \"a b")[1], ("\"a b".into(), Role::DoubleQuote));
        // A backslash inside double quotes escapes the quote that would have closed them. The
        // string is split at the escape now, so the pieces are what to check.
        let pieces: String = roles(r#"echo "a\"b""#)[1..]
            .iter()
            .map(|(text, _)| text.as_str())
            .collect();
        assert_eq!(pieces, r#""a\"b""#);
    }

    #[test]
    fn escapes_are_their_own_span() {
        let spans = roles(r"echo a\ b");
        assert!(
            spans.iter().any(|(t, r)| t == r"\ " && *r == Role::Escape),
            "{spans:?}"
        );
        // A trailing backslash does not run past the end of the line.
        assert_eq!(
            lex(r"echo \").iter().map(|s| s.text.len()).sum::<usize>(),
            6
        );
    }
}
