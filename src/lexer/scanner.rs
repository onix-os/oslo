//! The lexer core: cursor, token dispatch and operator scanning.
//!
//! Word scanning lives in [`super::quoting`] and `$`-expansions in [`super::expansion`];
//! all three write `impl Lexer` blocks against the struct defined here.

use crate::error::{Result, ShellError};
use crate::lexer::token::Token;

/// How deeply `$( )`, `${ }`, `"…"` and backticks may nest inside a single word.
///
/// The raw scanners below recurse once per nesting level, so input like `$($($(…` would abort the
/// process on a stack overflow before any diagnostic could be printed — the same failure mode
/// [`crate::parser::nesting`] exists to prevent. Real words nest a handful of levels at most.
const MAX_WORD_NESTING: usize = 64;

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
            // `{` and `}` are deliberately absent: they are not operator characters in the shell
            // grammar, they are reserved *words*, recognised only when they stand alone. Scanning
            // them as operators cut `{a,b}` off from the rest of its word, and since the caller
            // (word re-lexing) then falls back to treating the whole text as one literal, it also
            // silently disabled every expansion in a word that happened to start with a brace.
            _ => return Ok(None),
        };

        Ok(Some(token))
    }
}

/// Raw, quote-aware scanning of bracketed constructs.
///
/// These copy source text verbatim rather than decomposing it into [`crate::ast::WordPart`]s: a
/// `$( )` body is re-parsed as a whole script and a `${ }` body is picked apart by the
/// parameter-expansion parser, and both need the quotes the user actually wrote. What the quote
/// tracking buys is knowing which delimiters *count* — the `)` in `echo "$(echo "a)b")"` is data,
/// not the end of the substitution.
impl Lexer<'_> {
    /// Copy up to the delimiter matching an `open` that was just consumed, excluding it.
    pub(super) fn scan_raw_delimited(
        &mut self,
        open: char,
        close: char,
        what: &str,
        depth: usize,
    ) -> Result<String> {
        if depth > MAX_WORD_NESTING {
            return Err(ShellError::SyntaxError(format!("{what} nested too deeply")));
        }

        let mut out = String::new();
        let mut level = 1usize;

        while let Some(ch) = self.current_char() {
            // A nested expansion is followed into whatever brackets *this* scan is counting, so
            // the `}` in `${x:-$(echo })}` belongs to the substitution and not to the operand.
            if ch == '$' && self.copy_dollar_group(&mut out, depth)? {
                continue;
            }
            if ch == close {
                self.advance();
                level -= 1;
                if level == 0 {
                    return Ok(out);
                }
                out.push(ch);
                continue;
            }
            if ch == open {
                self.advance();
                level += 1;
                out.push(ch);
                continue;
            }

            match ch {
                '\'' => {
                    self.advance();
                    out.push('\'');
                    self.copy_single_quoted(&mut out)?;
                }
                '"' => {
                    self.advance();
                    out.push('"');
                    self.copy_double_quoted(&mut out, depth + 1)?;
                }
                '`' => {
                    self.advance();
                    out.push('`');
                    self.copy_backquoted(&mut out)?;
                }
                '\\' => {
                    self.advance();
                    out.push('\\');
                    if let Some(next) = self.advance() {
                        out.push(next);
                    }
                }
                _ => {
                    out.push(ch);
                    self.advance();
                }
            }
        }

        Err(ShellError::SyntaxError(format!("Unterminated {what}")))
    }

    /// Copy a `$( … )` or `${ … }` starting at the cursor, brackets included.
    ///
    /// Returns `false` — having consumed nothing — when the `$` starts something else, so callers
    /// can fall through to their ordinary handling of it.
    fn copy_dollar_group(&mut self, out: &mut String, depth: usize) -> Result<bool> {
        let (open, close, what) = match self.peek_char() {
            Some('(') => ('(', ')', "command substitution"),
            Some('{') => ('{', '}', "parameter expansion"),
            _ => return Ok(false),
        };

        self.advance();
        self.advance();
        out.push('$');
        out.push(open);
        let inner = self.scan_raw_delimited(open, close, what, depth + 1)?;
        out.push_str(&inner);
        out.push(close);
        Ok(true)
    }

    /// Copy through the closing `'`. Nothing inside is special, not even a backslash.
    fn copy_single_quoted(&mut self, out: &mut String) -> Result<()> {
        while let Some(ch) = self.advance() {
            out.push(ch);
            if ch == '\'' {
                return Ok(());
            }
        }
        Err(ShellError::SyntaxError(
            "Unterminated single quote".to_string(),
        ))
    }

    /// Copy through the closing `"`.
    ///
    /// Expansions stay live inside double quotes, so a `$( )` or `${ }` opened here has to be
    /// followed into: `"$(echo ")")"` closes on the *third* quote, not the second.
    fn copy_double_quoted(&mut self, out: &mut String, depth: usize) -> Result<()> {
        while let Some(ch) = self.current_char() {
            match ch {
                '"' => {
                    self.advance();
                    out.push('"');
                    return Ok(());
                }
                '\\' => {
                    self.advance();
                    out.push('\\');
                    if let Some(next) = self.advance() {
                        out.push(next);
                    }
                }
                '`' => {
                    self.advance();
                    out.push('`');
                    self.copy_backquoted(out)?;
                }
                '$' if self.copy_dollar_group(out, depth)? => {}
                _ => {
                    out.push(ch);
                    self.advance();
                }
            }
        }

        Err(ShellError::SyntaxError(
            "Unterminated double quote".to_string(),
        ))
    }

    /// Copy through the closing backquote. Only `` \` ``, `\\` and `\$` are escapes here.
    fn copy_backquoted(&mut self, out: &mut String) -> Result<()> {
        while let Some(ch) = self.current_char() {
            self.advance();
            out.push(ch);
            if ch == '`' {
                return Ok(());
            }
            if ch == '\\'
                && matches!(self.current_char(), Some('`') | Some('\\') | Some('$'))
                && let Some(next) = self.advance()
            {
                out.push(next);
            }
        }
        Err(ShellError::SyntaxError(
            "Unterminated backquote".to_string(),
        ))
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
