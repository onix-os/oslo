//! Parsing: shell source text in, [`crate::ast`] out.
//!
//! There is exactly one parser — `brush_parser`, adapted in [`brush_adapter`]. rush used to carry
//! a second, hand-written one as a fallback for anything brush rejected, and that design turned
//! every gap in the adapter into a silent reinterpretation of the whole program: the fallback had
//! no here-document support, so it parsed heredoc *bodies* as commands and ran them. Inert data
//! became code. A single parser that reports its errors is worth far more than a second one that
//! guesses.
pub mod brush_adapter;
pub mod nesting;

pub use brush_adapter::parse_bash_script;
