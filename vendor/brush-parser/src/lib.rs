//! Implements a tokenizer and parsers for POSIX / bash shell syntax.

// Vendored code is not restyled — see vendor/README.md.
#![allow(clippy::collapsible_if)]
// TODO(unwrap): remove or scope this allow attribute
#![allow(clippy::unwrap_used)]

pub mod arithmetic;
pub mod ast;
pub mod pattern;
pub mod prompt;
pub mod readline_binding;
pub mod test_command;
pub mod word;

mod error;
mod memo;
mod parser;
mod source;
mod tokenizer;

pub use error::{
    BindingParseError, ParseError, ParseErrorLocation, TestCommandParseError, WordParseError,
};

#[cfg(feature = "diagnostics")]
pub use error::miette::PrettyError;

pub use parser::{Parser, ParserImpl, ParserOptions, SourceInfo, parse_tokens};

pub use source::{SourcePosition, SourcePositionOffset, SourceSpan};
pub use tokenizer::{
    Token, TokenLocation, TokenizerError, TokenizerOptions, tokenize_str,
    tokenize_str_with_options, uncached_tokenize_str, unquote_str,
};
