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
pub(crate) const KEYWORDS: [&str; 17] = [
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "function", "select", "time", "coproc",
];

/// Whether `word` is one of the shell's own reserved words.
///
/// Shared with the ghost, which must not offer to *complete* one: a closing keyword lands in
/// command position, so `fi` was being extended into a name that only looks like a command.
pub(crate) fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

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

        // **`\rm` and `\\rm` are commands, not escapes.** A leading backslash on the command word
        // is oslo's way of asking for the thing its own name is standing in front of — `\cmd`
        // skips the alias, the function and the builtin, `\\cmd` skips only the builtin. Lexed as
        // an ordinary escape, `\rm` came out as a `\r` span and an `m` span: the first letter took
        // the escape colour and the rest read as an argument. See `exec::simple::escape`.
        if ch == '\\'
            && command_position
            && let Some(len) = escaped_command_len(rest)
        {
            push(&mut spans, &rest[..len], Role::CommandWord);
            i += len;
            command_position = false;
            continue;
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
        let end = rest.find(ends_word).unwrap_or(rest.len());
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

/// Where an ordinary word ends: the characters that cannot be part of one.
fn ends_word(c: char) -> bool {
    c.is_whitespace() || matches!(c, '|' | '&' | ';' | '<' | '>' | '\'' | '"' | '$' | '\\')
}

/// How many bytes of an escaped command word `rest` begins with, if it begins with one.
///
/// `\cmd` and `\\cmd` only, and only when a *name* follows: `\ file` is an escaped space and
/// `\$HOME` an escaped dollar, both of which are ordinary escapes wherever they appear. The
/// doubled form is tried first, or `\\rm` would be read as `\` + `\rm`.
fn escaped_command_len(rest: &str) -> Option<usize> {
    let after = rest
        .strip_prefix(r"\\")
        .or_else(|| rest.strip_prefix('\\'))?;
    if after.chars().next().is_none_or(ends_word) {
        return None;
    }
    let name = after.find(ends_word).unwrap_or(after.len());
    Some(rest.len() - after.len() + name)
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
#[path = "lex/tests.rs"]
mod tests;
