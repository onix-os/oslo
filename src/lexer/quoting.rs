//! Word scanning and quote handling.
//!
//! A shell word is a sequence of parts — literal text, single-quoted runs, double-quoted runs
//! (which may themselves contain expansions) — which the evaluator later expands in order.

use super::scanner::{Lexer, is_operator_char};
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

impl Lexer<'_> {
    pub(super) fn scan_word(&mut self) -> Result<Token> {
        let parts = self.scan_word_parts(false)?;

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
            if !to_eof && (ch.is_whitespace() || is_operator_char(ch)) {
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
        let mut parts = Vec::new();
        let mut current_lit = String::new();

        while let Some(ch) = self.current_char() {
            match ch {
                '"' => {
                    self.advance();
                    if !current_lit.is_empty() {
                        parts.push(WordPart::Literal(current_lit));
                    }
                    return Ok(parts);
                }
                '\\' => {
                    self.advance();
                    if let Some(next) = self.current_char() {
                        if matches!(next, '$' | '`' | '"' | '\\' | '\n') {
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

        Err(ShellError::SyntaxError(
            "Unterminated double quote".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::WordPart;
    use crate::lexer::{Lexer, Token};

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
}
