//! Tokenizing shell input.
//!
//! Split by what is being scanned: [`scanner`] holds the cursor and operator handling,
//! [`quoting`] scans words and quoted runs, [`expansion`] scans `$`-expansions.

pub mod expansion;
pub mod quoting;
pub mod scanner;
pub mod token;

pub use scanner::Lexer;
pub use token::Token;
