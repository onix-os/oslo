//! Word scanning and quote handling.
//!
//! A shell word is a sequence of parts — literal text, single-quoted runs, double-quoted runs
//! (which may themselves contain expansions) — which the evaluator later expands in order.

use super::scanner::{Lexer, is_blank, is_operator_char};
use crate::lexer::token::Token;
use oslo_base::ast::{Word, WordPart};
use oslo_base::error::{Result, ShellError};

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
#[path = "quoting/tests.rs"]
mod tests;
