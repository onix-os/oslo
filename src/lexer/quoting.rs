//! Word scanning and quote handling.
//!
//! A shell word is a sequence of parts — literal text, single-quoted runs, double-quoted runs
//! (which may themselves contain expansions) — which the evaluator later expands in order.

use super::scanner::{Lexer, is_operator_char};
use crate::ast::{Word, WordPart};
use crate::error::{Result, ShellError};
use crate::lexer::token::Token;

impl Lexer<'_> {
    pub(super) fn scan_word(&mut self) -> Result<Token> {
        let mut parts = Vec::new();
        let mut current_lit = String::new();
        let mut is_io_number = true;

        while let Some(ch) = self.current_char() {
            if ch.is_whitespace() || is_operator_char(ch) {
                break;
            }

            if !ch.is_ascii_digit() {
                is_io_number = false;
            }

            match ch {
                '\\' => {
                    self.advance();
                    if let Some(escaped) = self.current_char() {
                        current_lit.push(escaped);
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
                    let exp = self.scan_dollar_expansion()?;
                    parts.push(exp);
                }
                '~' if parts.is_empty() && current_lit.is_empty() => {
                    self.advance();
                    let mut user = String::new();
                    while let Some(c) = self.current_char() {
                        if c.is_alphanumeric() || c == '_' || c == '-' {
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
            parts.push(WordPart::Literal(current_lit.clone()));
        }

        // A bare run of digits immediately followed by `<` or `>` is an fd number, as in `2>`.
        if is_io_number
            && !current_lit.is_empty()
            && parts.len() == 1
            && let Some(op_ch) = self.current_char()
            && (op_ch == '<' || op_ch == '>')
            && let Ok(num) = current_lit.parse::<i32>()
        {
            return Ok(Token::IoNumber(num));
        }

        let word = Word { parts };

        // Check if word is a reserved word
        if word.parts.len() == 1
            && let WordPart::Literal(ref s) = word.parts[0]
        {
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

        Ok(Token::Word(word))
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

    fn scan_double_quote(&mut self) -> Result<Vec<WordPart>> {
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
                    let exp = self.scan_dollar_expansion()?;
                    parts.push(exp);
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
