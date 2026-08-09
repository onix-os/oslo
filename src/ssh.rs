//! SSH, behind the `ssh` feature.
//!
//! Compiled only with `--features ssh`, which is off by default. A build without it is byte for
//! byte the shell that existed before this file: no `maki`, no `tokio`, no extra crate in the
//! lockfile's resolved graph for the default target.
//!
//! # Why this module exists before it does anything
//!
//! To hold the decision. The dependency was chosen by measurement rather than by reputation, and
//! the measurements are worth keeping next to the code that depends on them:
//!
//! | build | static musl binary | crates | C compiled |
//! |---|---|---|---|
//! | oslo, default | 5.57 MB | 142 | no |
//! | + `maki` | 6.15 MB | 225 | **no** |
//! | + `russh`, `ring` backend | 7.60 MB | 273 | yes |
//! | + `russh`, `ring` + `rsa` | 7.95 MB | 281 | yes |
//! | + `russh`, default backend | *does not link* | — | yes |
//!
//! `russh`'s default `aws-lc-rs` backend fails at link time against `x86_64-unknown-linux-musl`:
//! its C objects reference `__memcpy_chk`, `__vfprintf_chk` and `__isoc23_sscanf`, which are glibc
//! fortify symbols that musl does not have. Its `ring` backend links, but still runs a C compiler —
//! which is the reason `mlua` and `turso` are not here either.
//!
//! `ssh-rs` describes itself as a Rust implementation and pulls both `cc` and `ring`.
//!
//! # What is not decided
//!
//! **The async boundary.** `maki` is asynchronous and this shell is not. `tokio` was removed once
//! already, when `turso` went, because it existed only to bridge an async API to a synchronous
//! REPL — and every `OnceLock<Runtime>` and `block_on` went with it. Whether that bridge lives at
//! one edge or leaks inward is the thing to settle before any of this is switched on by default.
//!
//! **The crypto is unaudited**, by its own README, as are the RustCrypto crates beneath it. That is
//! acceptable for an opt-in feature and is not obviously acceptable for `/bin/sh`.

/// The client library, re-exported so the fork is reached through one name.
///
/// Everything that uses SSH goes through this module rather than naming `maki` directly — the same
/// rule `oslo-base/src/track/kv/` states for `tagdata`: callers use this boundary instead of the
/// dependency directly.
pub use maki;
