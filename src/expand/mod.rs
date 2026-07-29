//! Word expansion.
//!
//! Split by expansion step so each stage owns one file: [`word`] runs the pipeline and defines the
//! quoting provenance every other stage consults, [`brace`] runs first and splits one word into
//! several, [`param`] resolves `${...}`, [`fields`] applies IFS, [`glob`] applies pathname
//! expansion, [`tilde`] resolves `~`.

pub mod arithmetic;
pub mod brace;
pub mod fields;
pub mod glob;
pub mod param;
pub mod tilde;
pub mod word;

pub use word::{
    Field, Origin, Run, expand_word, expand_word_fields, expand_word_part, expand_word_to_string,
};
