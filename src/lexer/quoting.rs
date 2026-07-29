//! Word scanning and quote handling.
//!
//! A shell word is a sequence of parts — literal text, single-quoted runs, double-quoted runs
//! (which may themselves contain expansions) — which the evaluator later expands in order.

use super::scanner::{Lexer, is_blank, is_operator_char};
use crate::ast::{Word, WordPart};
use crate::error::{Result, ShellError};
use crate::lexer::token::Token;

/// Re-lex an operand that is already known to be exactly one word.
///
/// `${x:-$HOME}` stores its default as source text, and the expander needs it as parts. The
/// difference from [`Lexer::scan_word`] is that nothing here terminates the word: a space in
/// `${x:-a b}` is part of the operand, not a separator between two of them.
pub(super) fn parse_word_source(source: &str) -> Result<Word> {
    let mut lexer = Lexer::new(source);
    Ok(Word {
        parts: lexer.scan_word_parts(true)?,
    })
}

/// Split an unquoted here-document body into the parts expansion will run over.
///
/// A body is not a word and must not be scanned like one. The word scanner terminates nothing
/// when asked to run to end of input, but it still treats `'` and `"` as quotes and `~` as a
/// tilde prefix; inside a heredoc all three are ordinary characters, so `echo "it's fine"` in a
/// body would raise "unterminated single quote" on a script bash runs without complaint.
///
/// The live set is exactly the one double quotes have — `$`, `` ` ``, and a backslash before
/// `$`, `` ` ``, `\` or a newline — which is why this shares the double-quote scanner's shape
/// and differs only in having no closing delimiter to look for.
pub fn parse_heredoc_body(source: &str) -> Result<Word> {
    let mut lexer = Lexer::new(source);
    Ok(Word {
        parts: lexer.scan_heredoc_parts()?,
    })
}

impl Lexer<'_> {
    pub(super) fn scan_word(&mut self) -> Result<Token> {
        let start = self.offset();
        let parts = self.scan_word_parts(false)?;

        // A word with no parts that also consumed nothing is not a token, it is a stall. Refusing
        // it here bounds every loop over `next()` no matter which scanner disagreed; see
        // `Lexer::stalled_at`. `''` and `""` are not this case — both produce a part.
        if parts.is_empty() && self.offset() == start {
            return Err(self.stalled_at());
        }

        // A bare run of digits immediately followed by `<` or `>` is an fd number, as in `2>`.
        // One literal part and nothing else is what rules out `\2>` and `"2">`: any quoting at
        // all splits the run into something other than a single `Literal`.
        if let [WordPart::Literal(lit)] = parts.as_slice()
            && !lit.is_empty()
            && lit.bytes().all(|b| b.is_ascii_digit())
            && let Some(op_ch) = self.current_char()
            && (op_ch == '<' || op_ch == '>')
            && let Ok(num) = lit.parse::<i32>()
        {
            return Ok(Token::IoNumber(num));
        }

        // Check if word is a reserved word
        if let [WordPart::Literal(s)] = parts.as_slice() {
            match s.as_str() {
                "if" => return Ok(Token::If),
                "then" => return Ok(Token::Then),
                "else" => return Ok(Token::Else),
                "elif" => return Ok(Token::Elif),
                "fi" => return Ok(Token::Fi),
                "case" => return Ok(Token::Case),
                "esac" => return Ok(Token::Esac),
                "for" => return Ok(Token::For),
                "while" => return Ok(Token::While),
                "until" => return Ok(Token::Until),
                "do" => return Ok(Token::Do),
                "done" => return Ok(Token::Done),
                "in" => return Ok(Token::In),
                _ => {}
            }
        }

        Ok(Token::Word(Word { parts }))
    }

    /// Scan word parts up to the first separator, or to end of input when `to_eof`.
    fn scan_word_parts(&mut self, to_eof: bool) -> Result<Vec<WordPart>> {
        let mut parts = Vec::new();
        let mut current_lit = String::new();

        while let Some(ch) = self.current_char() {
            // `is_blank`, not `char::is_whitespace`: the two must name the same set as
            // `skip_whitespace` or a character in the gap ends the word here and is then skipped
            // by nobody, which is a lexer that cannot advance. `\n` is covered by
            // `is_operator_char`.
            if !to_eof && (is_blank(ch) || is_operator_char(ch)) {
                break;
            }

            match ch {
                '\\' => {
                    self.advance();
                    if let Some(escaped) = self.current_char() {
                        if !current_lit.is_empty() {
                            parts.push(WordPart::Literal(std::mem::take(&mut current_lit)));
                        }
                        // Not folded into `current_lit`: an escaped character is quoted, and the
                        // expander has to be able to tell it apart from text the user typed bare
                        // so that `echo \*` prints `*` instead of globbing.
                        parts.push(WordPart::Escaped(escaped.to_string()));
                        self.advance();
                    }
                }
                '\'' => {
                    self.advance();
                    if !current_lit.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut current_lit)));
                    }
                    let sq = self.scan_single_quote()?;
                    parts.push(WordPart::SingleQuoted(sq));
                }
                '"' => {
                    self.advance();
                    if !current_lit.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut current_lit)));
                    }
                    let dq_parts = self.scan_double_quote()?;
                    parts.push(WordPart::DoubleQuoted(dq_parts));
                }
                '$' => {
                    self.advance();
                    if !current_lit.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut current_lit)));
                    }
                    let exp = self.scan_dollar_expansion(false)?;
                    parts.push(exp);
                }
                '`' => {
                    if !current_lit.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut current_lit)));
                    }
                    parts.push(self.scan_backquote_substitution()?);
                }
                '~' if parts.is_empty() && current_lit.is_empty() => {
                    self.advance();
                    let mut user = String::new();
                    while let Some(c) = self.current_char() {
                        // `+` is here for `~+` (the current directory); `-` doubles as `~-` (the
                        // previous one) and as a character usernames are allowed to contain, so
                        // which of the two a run means is decided by expansion, not by the lexer.
                        if c.is_alphanumeric() || c == '_' || c == '-' || c == '+' {
                            user.push(c);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    parts.push(WordPart::Tilde(user));
                }
                _ => {
                    current_lit.push(ch);
                    self.advance();
                }
            }
        }

        if !current_lit.is_empty() {
            parts.push(WordPart::Literal(current_lit));
        }

        Ok(parts)
    }

    fn scan_single_quote(&mut self) -> Result<String> {
        let mut content = String::new();
        while let Some(ch) = self.current_char() {
            if ch == '\'' {
                self.advance();
                return Ok(content);
            }
            content.push(ch);
            self.advance();
        }
        Err(ShellError::SyntaxError(
            "Unterminated single quote".to_string(),
        ))
    }

    pub(super) fn scan_double_quote(&mut self) -> Result<Vec<WordPart>> {
        self.scan_quoted_run(Some('"'))
    }

    /// Scan a heredoc body: the same live set, no closing delimiter, EOF is the end.
    fn scan_heredoc_parts(&mut self) -> Result<Vec<WordPart>> {
        self.scan_quoted_run(None)
    }

    /// The run shared by double quotes and here-document bodies.
    ///
    /// `closing` is the character that ends the run, and it doubles as the extra character a
    /// backslash may escape — `\"` is a quote inside `"…"`, while in a heredoc body there is no
    /// such character and `\"` stays two characters, exactly as bash writes it out.
    ///
    /// `None` also means running off the end is not an error: a body ends where the delimiter
    /// line was, and the caller has already cut it there.
    fn scan_quoted_run(&mut self, closing: Option<char>) -> Result<Vec<WordPart>> {
        let mut parts = Vec::new();
        let mut current_lit = String::new();

        while let Some(ch) = self.current_char() {
            if Some(ch) == closing {
                self.advance();
                if !current_lit.is_empty() {
                    parts.push(WordPart::Literal(current_lit));
                }
                return Ok(parts);
            }
            match ch {
                '\\' => {
                    self.advance();
                    if let Some(next) = self.current_char() {
                        if matches!(next, '$' | '`' | '\\' | '\n') || Some(next) == closing {
                            if next != '\n' {
                                current_lit.push(next);
                            }
                            self.advance();
                        } else {
                            current_lit.push('\\');
                        }
                    } else {
                        current_lit.push('\\');
                    }
                }
                '$' => {
                    self.advance();
                    if !current_lit.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut current_lit)));
                    }
                    // `$'…'` and `$"…"` are inert inside double quotes — bash echoes `$'a\tb'`
                    // back unchanged — so the flag says so.
                    let exp = self.scan_dollar_expansion(true)?;
                    parts.push(exp);
                }
                // Backquotes stay live inside double quotes, unlike single quotes.
                '`' => {
                    if !current_lit.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut current_lit)));
                    }
                    parts.push(self.scan_backquote_substitution()?);
                }
                _ => {
                    current_lit.push(ch);
                    self.advance();
                }
            }
        }

        if closing.is_some() {
            return Err(ShellError::SyntaxError(
                "Unterminated double quote".to_string(),
            ));
        }
        if !current_lit.is_empty() {
            parts.push(WordPart::Literal(current_lit));
        }
        Ok(parts)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_heredoc_body;
    use crate::ast::WordPart;
    use crate::lexer::{Lexer, Token};

    fn body(src: &str) -> Vec<WordPart> {
        parse_heredoc_body(src)
            .unwrap_or_else(|e| panic!("heredoc body {src:?} did not scan: {e}"))
            .parts
    }

    fn parts(src: &str) -> Vec<WordPart> {
        match Lexer::new(src).next() {
            Ok(Token::Word(w)) => w.parts,
            other => panic!("expected a word from {src:?}, got {other:?}"),
        }
    }

    /// The whole point of the variant: an escaped character has to be distinguishable from text
    /// the user typed bare, or `echo \*` globs.
    #[test]
    fn a_backslash_escape_is_not_a_literal() {
        assert_eq!(parts("\\*"), vec![WordPart::Escaped("*".into())]);
        assert_eq!(
            parts("a\\ b"),
            vec![
                WordPart::Literal("a".into()),
                WordPart::Escaped(" ".into()),
                WordPart::Literal("b".into()),
            ]
        );
    }

    #[test]
    fn adjacent_escapes_stay_separate_parts() {
        assert_eq!(
            parts("\\*\\?"),
            vec![WordPart::Escaped("*".into()), WordPart::Escaped("?".into()),]
        );
    }

    /// `\if` is a word, not the reserved word `if`: the keyword check only fires on a lone literal.
    #[test]
    fn an_escaped_keyword_is_not_a_keyword() {
        assert_eq!(
            parts("\\if"),
            vec![WordPart::Escaped("i".into()), WordPart::Literal("f".into()),]
        );
    }

    /// A backslash inside double quotes is the double-quote rule's job; the run is quoted either
    /// way, so it stays a plain literal.
    #[test]
    fn double_quoted_escapes_are_still_literals() {
        assert_eq!(
            parts("\"a\\$b\""),
            vec![WordPart::DoubleQuoted(vec![WordPart::Literal(
                "a$b".into()
            )])]
        );
    }

    /// A digit run before `<` or `>` is still an fd number; a backslash anywhere rules that out.
    #[test]
    fn escaping_defeats_the_io_number_rule() {
        assert!(matches!(Lexer::new("2>").next(), Ok(Token::IoNumber(2))));
        assert!(matches!(Lexer::new("\\2>").next(), Ok(Token::Word(_))));
        assert!(matches!(Lexer::new("\"2\">").next(), Ok(Token::Word(_))));
    }

    // --- R2.3: backquote command substitution ---

    #[test]
    fn a_backquote_run_is_a_command_substitution() {
        assert_eq!(
            parts("`echo hi`"),
            vec![WordPart::CommandSubstitution("echo hi".into())]
        );
    }

    /// Backquotes are live inside double quotes, and adjacent to other parts of the same word.
    #[test]
    fn backquotes_are_scanned_in_every_live_context() {
        assert_eq!(
            parts("\"v: `echo q`\""),
            vec![WordPart::DoubleQuoted(vec![
                WordPart::Literal("v: ".into()),
                WordPart::CommandSubstitution("echo q".into()),
            ])]
        );
        assert_eq!(
            parts("a`f`b"),
            vec![
                WordPart::Literal("a".into()),
                WordPart::CommandSubstitution("f".into()),
                WordPart::Literal("b".into()),
            ]
        );
    }

    /// Inside backquotes exactly three escapes exist; every other backslash is data the inner
    /// parse still has to see, so `\n` must survive as two characters.
    #[test]
    fn backquote_escapes_are_stripped_only_for_the_three_special_characters() {
        assert_eq!(
            parts("`echo \\`x\\` \\$v \\\\ \\n`"),
            vec![WordPart::CommandSubstitution("echo `x` $v \\ \\n".into())]
        );
    }

    /// A single quote is not a quote inside backquotes as far as *this* scan is concerned, but an
    /// unterminated backquote is still an error rather than silent literal text.
    #[test]
    fn an_unterminated_backquote_is_a_syntax_error() {
        assert!(Lexer::new("`echo hi").next().is_err());
    }

    // --- R2.7: quote-aware `$( )` scanning ---

    #[test]
    fn a_quoted_paren_does_not_end_a_substitution() {
        assert_eq!(
            parts("\"$(echo \"a)b\")\""),
            vec![WordPart::DoubleQuoted(vec![WordPart::CommandSubstitution(
                "echo \"a)b\"".into()
            )])]
        );
        assert_eq!(
            parts("$(echo '(paren)')"),
            vec![WordPart::CommandSubstitution("echo '(paren)'".into())]
        );
    }

    /// `$((` is not always arithmetic: it is also a substitution whose first command is a
    /// subshell. What follows the balanced group decides which.
    #[test]
    fn a_leading_subshell_is_not_arithmetic() {
        assert_eq!(parts("$((1+2))"), vec![WordPart::Arithmetic("1+2".into())]);
        assert_eq!(
            parts("$(((1+2)*3))"),
            vec![WordPart::Arithmetic("(1+2)*3".into())]
        );
        assert_eq!(
            parts("$((echo a); echo b)"),
            vec![WordPart::CommandSubstitution("(echo a); echo b".into())]
        );
    }

    #[test]
    fn an_unterminated_substitution_is_a_syntax_error() {
        assert!(Lexer::new("$(echo hi").next().is_err());
        assert!(Lexer::new("${x:-y").next().is_err());
    }

    /// Round 11 A1. `skip_whitespace` stepped over these and `scan_word_parts` refused to consume
    /// them, so the lexer returned part-less words forever and every caller looping to `Eof` hung
    /// while allocating. They are ordinary word characters in bash, and now here.
    #[test]
    fn unicode_blanks_are_word_characters_not_separators() {
        for blank in [
            '\u{0b}', '\u{0c}', '\u{a0}', '\u{2028}', '\u{2007}', '\u{85}',
        ] {
            let src = format!("a{blank}b");
            assert_eq!(
                parts(&src),
                vec![WordPart::Literal(src.clone())],
                "U+{:04X} split a word",
                blank as u32
            );
        }
    }

    // --- R11.B2: here-document bodies ---

    /// The reason a body cannot go through the word scanner. All three of these end a word, or
    /// open a quote that never closes, and in a heredoc all three are ordinary characters.
    #[test]
    fn quotes_blanks_and_tildes_are_plain_text_in_a_body() {
        assert_eq!(
            body("it's \"fine\"\t~root *\n"),
            vec![WordPart::Literal("it's \"fine\"\t~root *\n".into())]
        );
        assert!(
            super::parse_word_source("it's fine").is_err(),
            "the word scanner accepts an unterminated quote, so this test proves nothing"
        );
    }

    /// The live set is the double-quoted one, so expansions still become parts.
    #[test]
    fn expansions_in_a_body_are_still_scanned() {
        assert_eq!(
            body("x=$v\n"),
            vec![
                WordPart::Literal("x=".into()),
                WordPart::Variable {
                    name: "v".into(),
                    expansion_type: crate::ast::ParamExpansion::Normal,
                },
                WordPart::Literal("\n".into()),
            ]
        );
        assert_eq!(
            body("$(echo a)`echo b`$((1+1))"),
            vec![
                WordPart::CommandSubstitution("echo a".into()),
                WordPart::CommandSubstitution("echo b".into()),
                WordPart::Arithmetic("1+1".into()),
            ]
        );
    }

    /// A backslash escapes `$`, a backtick, another backslash and a newline — and nothing else.
    /// `\"` in particular is two characters here, where inside double quotes it is one.
    #[test]
    fn only_four_escapes_exist_in_a_body() {
        assert_eq!(body("\\$v"), vec![WordPart::Literal("$v".into())]);
        assert_eq!(body("\\`x"), vec![WordPart::Literal("`x".into())]);
        assert_eq!(body("\\\\"), vec![WordPart::Literal("\\".into())]);
        assert_eq!(body("a\\\nb"), vec![WordPart::Literal("ab".into())]);
        assert_eq!(
            body("\\\"q\\\""),
            vec![WordPart::Literal("\\\"q\\\"".into())]
        );
        assert_eq!(body("\\n\\t"), vec![WordPart::Literal("\\n\\t".into())]);
    }

    /// A body ends where its delimiter line was, which the caller has already cut off. Running
    /// out of input is therefore the normal ending, not the error it is inside quotes.
    #[test]
    fn a_body_ends_at_end_of_input_rather_than_erroring() {
        assert_eq!(body(""), vec![]);
        assert_eq!(
            body("no trailing newline"),
            vec![WordPart::Literal("no trailing newline".into())]
        );
        assert!(
            Lexer::new("unterminated").scan_double_quote().is_err(),
            "the shared scanner must still refuse an unterminated double quote"
        );
    }

    /// The property that makes the hang impossible rather than merely fixed: whatever the input,
    /// `next()` either consumes something or reaches `Eof`/an error.
    #[test]
    fn every_token_consumes_at_least_one_character() {
        for src in [
            "a\u{0b}b",
            "echo a\u{a0}b",
            "\u{2028}",
            "\u{a0}",
            "\u{0c}\u{0b}\u{85}",
            "a b\tc\rd\ne",
            "''",
            "\"\"",
            "x=1\u{0b}2",
            "~\u{a0}",
        ] {
            let mut lexer = Lexer::new(src);
            // One token per character is the ceiling; anything past it is a scanner that stopped
            // advancing, which is the bug this bounds rather than a test of tokenization.
            for _ in 0..=src.chars().count() {
                let before = lexer.offset();
                match lexer.next() {
                    Ok(Token::Eof) | Err(_) => break,
                    Ok(tok) => assert!(
                        lexer.offset() > before,
                        "{src:?}: {tok:?} consumed nothing at offset {before}"
                    ),
                }
            }
        }
    }
}
