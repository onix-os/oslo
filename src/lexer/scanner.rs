//! The lexer core: cursor, token dispatch and operator scanning.
//!
//! Word scanning lives in [`super::quoting`] and `$`-expansions in [`super::expansion`];
//! all three write `impl Lexer` blocks against the struct defined here.

use crate::error::Result;
use crate::lexer::token::Token;

#[allow(clippy::should_implement_trait, clippy::collapsible_if)]
pub struct Lexer<'a> {
    _input: &'a str,
    chars: Vec<char>,

    pos: usize,
    peeked: Option<Token>,
}

#[allow(clippy::should_implement_trait, clippy::collapsible_if)]
impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            _input: input,
            chars: input.chars().collect(),

            pos: 0,
            peeked: None,
        }
    }

    pub(super) fn is_eof(&self) -> bool {
        self.pos >= self.chars.len()
    }

    pub(super) fn current_char(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    pub(super) fn peek_char(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    pub(super) fn advance(&mut self) -> Option<char> {
        if self.pos < self.chars.len() {
            let ch = self.chars[self.pos];
            self.pos += 1;
            Some(ch)
        } else {
            None
        }
    }

    pub fn peek(&mut self) -> Result<&Token> {
        if self.peeked.is_none() {
            let tok = self.next_token()?;
            self.peeked = Some(tok);
        }
        Ok(self.peeked.as_ref().unwrap())
    }

    pub fn next(&mut self) -> Result<Token> {
        if let Some(tok) = self.peeked.take() {
            Ok(tok)
        } else {
            self.next_token()
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char() {
            if ch == ' ' || ch == '\t' || ch == '\r' {
                self.advance();
            } else if ch == '\\' && self.peek_char() == Some('\n') {
                // Line continuation
                self.advance();
                self.advance();
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<Token> {
        self.skip_whitespace();

        if self.is_eof() {
            return Ok(Token::Eof);
        }

        let ch = self.current_char().unwrap();

        // Newline
        if ch == '\n' {
            self.advance();
            return Ok(Token::Newline);
        }

        // Comment `#`
        if ch == '#' {
            while let Some(c) = self.current_char() {
                if c == '\n' {
                    break;
                }
                self.advance();
            }
            return self.next_token();
        }

        // Operators
        if let Some(tok) = self.scan_operator()? {
            return Ok(tok);
        }

        // Word or IoNumber
        self.scan_word()
    }

    fn scan_operator(&mut self) -> Result<Option<Token>> {
        let ch = match self.current_char() {
            Some(c) => c,
            None => return Ok(None),
        };

        let next = self.peek_char();

        let token = match (ch, next) {
            ('|', Some('|')) => {
                self.advance();
                self.advance();
                Token::OrIf
            }
            ('|', _) => {
                self.advance();
                Token::Pipe
            }
            ('&', Some('&')) => {
                self.advance();
                self.advance();
                Token::AndIf
            }
            ('&', _) => {
                self.advance();
                Token::Amp
            }
            (';', Some(';')) => {
                self.advance();
                self.advance();
                Token::Dsemi
            }
            (';', _) => {
                self.advance();
                Token::Semicolon
            }
            ('<', Some('<')) => {
                self.advance();
                self.advance();
                if self.current_char() == Some('-') {
                    self.advance();
                    Token::DlessDash
                } else {
                    Token::Dless
                }
            }
            ('<', Some('&')) => {
                self.advance();
                self.advance();
                Token::LessAnd
            }
            ('<', Some('>')) => {
                self.advance();
                self.advance();
                Token::LessGreat
            }
            ('<', _) => {
                self.advance();
                Token::Less
            }
            ('>', Some('>')) => {
                self.advance();
                self.advance();
                Token::Dgreat
            }
            ('>', Some('&')) => {
                self.advance();
                self.advance();
                Token::GreatAnd
            }
            ('>', Some('|')) => {
                self.advance();
                self.advance();
                Token::Clobber
            }
            ('>', _) => {
                self.advance();
                Token::Great
            }
            ('(', _) => {
                self.advance();
                Token::LParen
            }
            (')', _) => {
                self.advance();
                Token::RParen
            }
            ('{', _) => {
                self.advance();
                Token::LBrace
            }
            ('}', _) => {
                self.advance();
                Token::RBrace
            }
            _ => return Ok(None),
        };

        Ok(Some(token))
    }
}

pub(super) fn is_operator_char(ch: char) -> bool {
    matches!(ch, '|' | '&' | ';' | '<' | '>' | '(' | ')' | '\n')
}

pub(super) fn is_var_name_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

pub(super) fn is_special_param(ch: char) -> bool {
    matches!(ch, '?' | '$' | '!' | '#' | '*' | '@' | '0'..='9')
}
