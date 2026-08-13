//! This crate contains common structs and functions used across the `age` crates.
//!
//! You are probably looking for the [`age`](https://crates.io/crates/age) crate
//! itself. You should only need to directly depend on this crate if you are
//! implementing a custom recipient type.
//!
//! Vendored code is not restyled — see `vendor/README.md`. The allowances below are for the lints
//! oslo runs at `-D warnings` and upstream does not: what is unused here is unused only because the
//! recipient types and features oslo cannot use have been taken out, and restyling somebody else's
//! crate to silence a lint is how a fork stops being rebasable.
#![allow(unused_imports, unused_variables, dead_code, unexpected_cfgs)]
#![allow(clippy::all, clippy::pedantic)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Catch documentation errors caused by code changes.
#![deny(rustdoc::broken_intra_doc_links)]

// Re-export crates that are used in our public API.
pub use secrecy;

pub mod format;
pub mod io;
pub mod primitives;
