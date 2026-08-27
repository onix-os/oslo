//! Where a word at the prompt starts, what it means once quoting is stripped, and how to put a
//! completion back into the line.
//!
//! The prompt has to answer three questions about the text left of the cursor, and all three are
//! the same scan: which byte does the word being completed start at, is that word in command
//! position, and is the cursor inside an open quote. Splitting on whitespace cannot answer any of
//! them — `echo "My Fi` is one word, not three, and `true && ec` is a command position even
//! though it is not at column zero.

/// The quoting state the cursor sits in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Quote {
    /// Not inside quotes; shell-special characters must be backslash-escaped.
    #[default]
    None,
    /// Inside `'…`; only `'` itself is special.
    Single,
    /// Inside `"…`; `"`, `\`, `$` and a backquote are special.
    Double,
}

/// The word under the cursor, with everything the completer needs to replace it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word<'a> {
    /// Byte offset in the original line where the word begins — the opening quote, if any.
    pub start: usize,
    /// The raw text between `start` and the cursor, exactly as typed.
    pub text: &'a str,
    /// `text` with quotes and escapes resolved: what a filename actually has to start with.
    pub stem: String,
    /// The quoting still open at the cursor.
    pub quote: Quote,
    /// Whether a command name belongs here rather than an argument.
    pub command_position: bool,
    /// The completed words of this command, before the one under the cursor.
    ///
    /// Raw text, still quoted: the spec lookup wants `git`/`commit`, and unquoting every word on
    /// every keystroke would allocate for nothing. Redirection targets are left out, so
    /// `git >log commit -<TAB>` still knows it is completing options of `git commit`.
    pub prior_words: Vec<&'a str>,
    /// How many bytes at the front of `stem` are **context rather than text to replace**.
    ///
    /// Zero for an ordinary word, where `start` points at the beginning of `stem` and a completion
    /// replaces the whole of it. It is not zero when the word has been retargeted at a piece of
    /// itself: `rm /dir/{alpha,be` completes `be` against `/dir/`, so the stem has to be `/dir/be`
    /// for the directory to be read — and `start` points at the `be`, because `/dir/{alpha,` is
    /// already on the line.
    ///
    /// **Getting this wrong writes the directory twice.** A candidate built as a whole path and
    /// inserted at `start` gave `rm /dir/{alpha,/dir/beta`, which is a different and longer path
    /// than the one that was typed — silently, on a command whose documented example is `rm`.
    pub carried: usize,
    /// What was cut off the front of this word, break character and all — `--file=` of
    /// `--file=arch`, `host:` of `host:/pa`.
    ///
    /// **Because `=` ends a word and the flag before it is the question.** `after_break` retargets
    /// the word at what follows the break, which is right for `dd if=/dev/ur` and loses the one
    /// fact a spec needs: which flag is being given a value. Kept here rather than recovered from
    /// the line, because by then `start` points past it and the line is somebody else's.
    pub prefix: &'a str,
}

/// Words after which bash expects another command rather than an argument.
///
/// `then`/`do`/`else` are the reason `if grep -q x f; then ec<TAB>` has to complete commands:
/// the nearest separator is the `;`, but the word after it is a reserved word, not the command.
const COMMAND_INTRODUCERS: &[&str] = &[
    "!", "time", "if", "then", "elif", "else", "while", "until", "do", "{", "}",
];

/// Characters that end a word without starting a new command: redirections.
///
/// Kept apart from the separators below because the word after `>` is a filename. Treating them
/// alike would make `sort < <TAB>` offer command names.
fn is_redirect(c: char) -> bool {
    matches!(c, '<' | '>')
}

/// Characters that end a word *and* begin a new command.
fn is_command_separator(c: char) -> bool {
    matches!(c, '|' | '&' | ';' | '(' | ')' | '`')
}

/// Scan `line[..pos]` and describe the word the cursor is in.
///
/// The cursor position is clamped to a character boundary so a caller that computed it from a
/// grapheme count cannot panic us.
pub fn current_word(line: &str, pos: usize) -> Word<'_> {
    let pos = clamp_boundary(line, pos);
    let sub = &line[..pos];
    let mut scan = Scan::new(sub);

    let mut it = sub.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        match scan.quote {
            Quote::Single => {
                if c == '\'' {
                    scan.quote = Quote::None;
                }
                continue;
            }
            Quote::Double => {
                // Inside double quotes a backslash escapes the next character, so `"a\"b` is one
                // word with an open quote — not a closed one.
                if c == '\\' {
                    it.next();
                } else if c == '"' {
                    scan.quote = Quote::None;
                }
                continue;
            }
            Quote::None => {}
        }

        if c == '\\' {
            // `echo don\'t`: the backslash and the character after it are ordinary word text.
            // The old validator counted that quote and wedged the prompt forever.
            scan.word_start.get_or_insert(i);
            it.next();
            continue;
        }

        if c == '\'' || c == '"' {
            scan.word_start.get_or_insert(i);
            scan.quote = if c == '\'' {
                Quote::Single
            } else {
                Quote::Double
            };
            continue;
        }

        if c.is_whitespace() {
            scan.finish_word(i);
            if c == '\n' {
                scan.start_command();
            }
            continue;
        }

        if is_redirect(c) {
            scan.finish_word(i);
            scan.after_redirect = true;
            continue;
        }

        if is_command_separator(c) {
            scan.finish_word(i);
            scan.start_command();
            continue;
        }

        scan.word_start.get_or_insert(i);
    }

    let start = scan.word_start.unwrap_or(pos);
    let text = &sub[start..];
    let command_position = if scan.after_redirect {
        false
    } else if scan.words_in_cmd == 0 {
        true
    } else {
        scan.prior_words
            .last()
            .is_some_and(|w| COMMAND_INTRODUCERS.contains(w))
    };

    Word {
        start,
        text,
        stem: unquote(text),
        quote: scan.quote,
        command_position,
        prior_words: scan.prior_words,
        // The whole word is being replaced; nothing in front of it is context.
        carried: 0,
        prefix: "",
    }
}

/// Mutable state of the left-to-right scan, kept together so the word-ending rules live in one
/// place instead of being repeated at each of the three boundary kinds.
struct Scan<'a> {
    sub: &'a str,
    quote: Quote,
    word_start: Option<usize>,
    /// Completed words since the last command separator. Zero means the next word names a command.
    words_in_cmd: usize,
    prior_words: Vec<&'a str>,
    /// Set by `<`/`>`: the next word is a redirection target, not a command and not an argument.
    after_redirect: bool,
}

impl<'a> Scan<'a> {
    fn new(sub: &'a str) -> Self {
        Self {
            sub,
            quote: Quote::None,
            word_start: None,
            words_in_cmd: 0,
            prior_words: Vec::new(),
            after_redirect: false,
        }
    }

    fn finish_word(&mut self, end: usize) {
        let Some(s) = self.word_start.take() else {
            return;
        };
        if self.after_redirect {
            // A redirection target is not part of the command's word list: in `>out cmd`, `cmd`
            // is still the command name.
            self.after_redirect = false;
            return;
        }
        let text = &self.sub[s..end];
        // `FOO=1 bar` runs `bar`: a leading assignment does not consume the command slot.
        if self.words_in_cmd == 0 && is_assignment(text) {
            return;
        }
        self.words_in_cmd += 1;
        self.prior_words.push(text);
    }

    fn start_command(&mut self) {
        self.words_in_cmd = 0;
        self.prior_words.clear();
        self.after_redirect = false;
    }
}

/// Whether `word` is a variable assignment prefix, i.e. `NAME=` followed by anything.
fn is_assignment(word: &str) -> bool {
    let Some(eq) = word.find('=') else {
        return false;
    };
    let name = &word[..eq];
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The historical two-value form, kept because it is the shape the tests and the hinter want.
///
/// Unlike the version this replaces, the returned slice starts at the opening quote when the
/// cursor is inside one, so `echo "My Fi<TAB>` completes `My Fi` rather than `Fi`.
pub fn extract_current_word(line: &str, pos: usize) -> (usize, &str) {
    let word = current_word(line, pos);
    (word.start, word.text)
}

/// Resolve quotes and backslashes: the value the shell would see for this word.
pub fn unquote(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quote = Quote::None;
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match quote {
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                } else {
                    out.push(c);
                }
            }
            Quote::Double => match c {
                '\\' => {
                    if let Some(&n) = it.peek() {
                        // Only these four are escapable inside double quotes; elsewhere the
                        // backslash is literal.
                        if matches!(n, '"' | '\\' | '$' | '`') {
                            out.push(n);
                            it.next();
                        } else {
                            out.push('\\');
                        }
                    } else {
                        out.push('\\');
                    }
                }
                '"' => quote = Quote::None,
                _ => out.push(c),
            },
            Quote::None => match c {
                '\\' => {
                    if let Some(n) = it.next() {
                        out.push(n);
                    }
                }
                '\'' => quote = Quote::Single,
                '"' => quote = Quote::Double,
                _ => out.push(c),
            },
        }
    }
    out
}

/// Characters that would change the meaning of a word if left bare.
fn needs_escape(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\''
                | '\\'
                | '$'
                | '`'
                | '&'
                | ';'
                | '|'
                | '<'
                | '>'
                | '('
                | ')'
                | '*'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '#'
                | '!'
                | '^'
        )
}

/// Render `value` so that typing it produces `value` again.
///
/// `quote` is the quoting the user already opened. Completing inside `"…` has to stay inside it —
/// closing and reopening would leave a stray quote behind the cursor.
pub fn quote_replacement(value: &str, quote: Quote) -> String {
    match quote {
        Quote::Single => {
            // A single-quoted string cannot contain `'`; the idiom is to close, escape, reopen.
            format!("'{}'", value.replace('\'', r"'\''"))
        }
        Quote::Double => {
            let mut out = String::with_capacity(value.len() + 2);
            out.push('"');
            for c in value.chars() {
                if matches!(c, '"' | '\\' | '$' | '`') {
                    out.push('\\');
                }
                out.push(c);
            }
            out.push('"');
            out
        }
        Quote::None => {
            if value.is_empty() {
                return "''".to_string();
            }
            let mut out = String::with_capacity(value.len());
            for (i, c) in value.char_indices() {
                // A leading `~` is the one special character we want to survive: escaping it
                // would turn `~/Documents` into a directory literally named `~`.
                if i == 0 && c == '~' {
                    out.push(c);
                    continue;
                }
                if needs_escape(c) {
                    out.push('\\');
                }
                out.push(c);
            }
            out
        }
    }
}

/// Command names appearing in `line`, in the order they would run.
///
/// Used to feed the frecency table from a line the user actually accepted. It is deliberately
/// this scanner and not the parser: an unparseable line still tells us what was meant, and the
/// prompt must never pay for a parse it does not need.
pub fn command_words(line: &str) -> Vec<String> {
    let mut names = Vec::new();
    // Ask the scanner about every position where a word has just ended: that is exactly the set
    // of complete words, and `command_position` then says whether it named a command.
    let mut open = false;
    for (i, c) in line.char_indices() {
        let boundary = c.is_whitespace() || is_redirect(c) || is_command_separator(c);
        if boundary {
            if open {
                push_command_word(line, i, &mut names);
                open = false;
            }
        } else {
            open = true;
        }
    }
    if open {
        push_command_word(line, line.len(), &mut names);
    }
    names
}

fn push_command_word(line: &str, end: usize, names: &mut Vec<String>) {
    let w = current_word(line, end);
    // Quoting is honoured here too, so `echo "a b"` does not report `a` as a command.
    if w.command_position && !w.stem.is_empty() && !is_assignment(&w.stem) {
        names.push(w.stem);
    }
}

fn clamp_boundary(line: &str, pos: usize) -> usize {
    let mut pos = pos.min(line.len());
    while pos > 0 && !line.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_word_at_start_is_a_command() {
        let w = current_word("ec", 2);
        assert_eq!(w.start, 0);
        assert_eq!(w.text, "ec");
        assert!(w.command_position);
    }

    #[test]
    fn word_after_operator_is_a_command() {
        for line in ["true && ec", "true || ec", "true; ec", "true | ec", "(ec"] {
            let w = current_word(line, line.len());
            assert!(w.command_position, "{line} should be a command position");
            assert_eq!(w.text, "ec", "{line}");
        }
    }

    #[test]
    fn word_after_reserved_word_is_a_command() {
        let w = current_word("if true; then ec", 16);
        assert!(w.command_position);
        assert_eq!(w.text, "ec");
    }

    #[test]
    fn redirection_target_is_not_a_command() {
        let w = current_word("echo hi > fo", 12);
        assert!(!w.command_position);
        assert_eq!(w.text, "fo");
    }

    #[test]
    fn second_word_is_not_a_command() {
        let w = current_word("echo he", 7);
        assert!(!w.command_position);
    }

    #[test]
    fn open_double_quote_starts_the_word() {
        let w = current_word("wc -c \"My Fi", 12);
        assert_eq!(w.start, 6);
        assert_eq!(w.text, "\"My Fi");
        assert_eq!(w.stem, "My Fi");
        assert_eq!(w.quote, Quote::Double);
    }

    #[test]
    fn open_single_quote_starts_the_word() {
        let w = current_word("cat 'a b", 8);
        assert_eq!(w.start, 4);
        assert_eq!(w.stem, "a b");
        assert_eq!(w.quote, Quote::Single);
    }

    #[test]
    fn backslash_escape_keeps_the_word_together() {
        let w = current_word("wc -c My\\ Fi", 12);
        assert_eq!(w.start, 6);
        assert_eq!(w.stem, "My Fi");
        assert_eq!(w.quote, Quote::None);
    }

    #[test]
    fn escaped_quote_does_not_open_a_quote() {
        let w = current_word("echo don\\'t", 11);
        assert_eq!(w.quote, Quote::None);
        assert_eq!(w.stem, "don't");
    }

    #[test]
    fn operator_inside_quotes_is_not_a_boundary() {
        let w = current_word("echo \"a | b", 11);
        assert_eq!(w.start, 5);
        assert_eq!(w.quote, Quote::Double);
        assert!(!w.command_position);
    }

    #[test]
    fn extract_current_word_agrees_with_current_word() {
        let (start, text) = extract_current_word("git comm", 8);
        assert_eq!((start, text), (4, "comm"));
    }

    #[test]
    fn unquoted_replacement_escapes_specials() {
        assert_eq!(
            quote_replacement("My File.txt", Quote::None),
            "My\\ File.txt"
        );
        assert_eq!(quote_replacement("a'b", Quote::None), "a\\'b");
        assert_eq!(quote_replacement("*.rs", Quote::None), "\\*.rs");
        assert_eq!(quote_replacement("~/bin/x", Quote::None), "~/bin/x");
    }

    #[test]
    fn quoted_replacement_stays_inside_its_quotes() {
        assert_eq!(quote_replacement("My File", Quote::Double), "\"My File\"");
        assert_eq!(quote_replacement("a\"b", Quote::Double), "\"a\\\"b\"");
        assert_eq!(quote_replacement("My File", Quote::Single), "'My File'");
        assert_eq!(quote_replacement("it's", Quote::Single), r"'it'\''s'");
    }

    #[test]
    fn escaping_round_trips_through_unquote() {
        for value in ["My File.txt", "a'b", "a\"b", "x$y", "a b|c", "*?[]"] {
            for quote in [Quote::None, Quote::Single, Quote::Double] {
                let rendered = quote_replacement(value, quote);
                assert_eq!(unquote(&rendered), value, "{value:?} as {quote:?}");
            }
        }
    }

    #[test]
    fn command_words_finds_every_command_position() {
        assert_eq!(command_words("echo hi"), vec!["echo"]);
        assert_eq!(command_words("ls -l | wc -l"), vec!["ls", "wc"]);
        assert_eq!(command_words("true && false"), vec!["true", "false"]);
        assert_eq!(command_words("echo hi > out"), vec!["echo"]);
        assert_eq!(command_words("FOO=1 bar"), vec!["bar"]);
    }
}
