use std::path::PathBuf;

use crate::ast;
use crate::tokenizer::{Token, TokenEndReason, Tokenizer, TokenizerOptions, Tokens};

pub mod peg;

/// Parser implementation to use
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Default)]
pub enum ParserImpl {
    /// PEG-based parser (token-based)
    #[default]
    Peg,
}

/// Options used to control the behavior of the parser.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct ParserOptions {
    /// Whether or not to enable extended globbing (a.k.a. `extglob`).
    pub enable_extended_globbing: bool,
    /// Whether or not to enable POSIX compliance mode.
    pub posix_mode: bool,
    /// Whether or not to enable maximal compatibility with the `sh` shell.
    pub sh_mode: bool,
    /// Whether or not to perform tilde expansion for tildes at the start of words.
    pub tilde_expansion_at_word_start: bool,
    /// Whether or not to perform tilde expansion for tildes after colons.
    pub tilde_expansion_after_colon: bool,
    /// Select the parser internal implementation
    pub parser_impl: ParserImpl,
}

impl Default for ParserOptions {
    fn default() -> Self {
        Self {
            enable_extended_globbing: true,
            posix_mode: false,
            sh_mode: false,
            tilde_expansion_at_word_start: true,
            tilde_expansion_after_colon: false,
            parser_impl: ParserImpl::default(),
        }
    }
}

impl ParserOptions {
    /// Returns the tokenizer options implied by these parser options.
    pub const fn tokenizer_options(&self) -> TokenizerOptions {
        TokenizerOptions {
            enable_extended_globbing: self.enable_extended_globbing,
            posix_mode: self.posix_mode,
            sh_mode: self.sh_mode,
        }
    }
}

/// Information about the source of tokens.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct SourceInfo {
    /// The source of the tokens.
    pub source: String,
}

impl From<PathBuf> for SourceInfo {
    fn from(path: PathBuf) -> Self {
        Self {
            source: path.to_string_lossy().to_string(),
        }
    }
}

/// Implements parsing for shell programs.
pub struct Parser<R: std::io::BufRead> {
    /// The reader to use for input
    reader: R,
    /// Parsing options
    options: ParserOptions,
}

impl<R: std::io::BufRead> Parser<R> {
    ///
    /// # Arguments
    ///
    /// * `reader` - The reader to use for input.
    /// * `options` - The options to use when parsing.
    pub fn new(reader: R, options: &ParserOptions) -> Self {
        Self {
            reader,
            options: options.clone(),
        }
    }

    /// Parses the input into an abstract syntax tree (AST) of a shell program.
    pub fn parse_program(&mut self) -> Result<ast::Program, crate::error::ParseError> {
        //
        // References:
        //   * https://www.gnu.org/software/bash/manual/bash.html#Shell-Syntax
        //   * https://mywiki.wooledge.org/BashParser
        //   * https://aosabook.org/en/v1/bash.html
        //   * https://pubs.opengroup.org/onlinepubs/9699919799/utilities/V3_chap02.html
        //
        match self.options.parser_impl {
            ParserImpl::Peg => {
                let tokens = self.tokenize()?;
                parse_tokens(&tokens, &self.options)
            }
        }
    }

    /// Parses a function definition body from the input. The body is expected to be
    /// preceded by "()", but no function name.
    pub fn parse_function_parens_and_body(
        &mut self,
    ) -> Result<ast::FunctionBody, crate::error::ParseError> {
        let tokens = self.tokenize()?;
        let parse_result =
            peg::token_parser::function_parens_and_body(&Tokens { tokens: &tokens }, &self.options);
        parse_result_to_error(parse_result, &tokens)
    }

    fn tokenize(&mut self) -> Result<Vec<Token>, crate::error::ParseError> {
        // First we tokenize the input, according to the policy implied by provided options.
        let mut tokenizer = Tokenizer::new(&mut self.reader, &self.options.tokenizer_options());

        let mut tokens = vec![];
        loop {
            let result = match tokenizer.next_token() {
                Ok(result) => result,
                Err(e) => {
                    return Err(crate::error::ParseError::Tokenizing {
                        inner: e,
                        position: tokenizer.current_location(),
                    });
                }
            };

            let reason = result.reason;
            if let Some(token) = result.token {
                tokens.push(token);
            }

            if matches!(reason, TokenEndReason::EndOfInput) {
                break;
            }
        }

        Ok(tokens)
    }
}

/// Parses a sequence of tokens into the abstract syntax tree (AST) of a shell program.
///
/// # Arguments
///
/// * `tokens` - The tokens to parse.
/// * `options` - The options to use when parsing.
pub fn parse_tokens(
    tokens: &[Token],
    options: &ParserOptions,
) -> Result<ast::Program, crate::error::ParseError> {
    let parse_result = peg::token_parser::program(&Tokens { tokens }, options);
    parse_result_to_error(parse_result, tokens)
}

fn parse_result_to_error<R>(
    parse_result: Result<R, ::peg::error::ParseError<usize>>,
    tokens: &[Token],
) -> Result<R, crate::error::ParseError>
where
    R: std::fmt::Debug,
{
    match parse_result {
        Ok(program) => Ok(program),
        Err(parse_error) => Err(crate::error::convert_peg_parse_error(&parse_error, tokens)),
    }
}
