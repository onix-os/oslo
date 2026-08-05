//! What ranking a Tab press costs, measured rather than reasoned about.
//!
//! The shape that matters is the real one: one short typed pattern scored against every executable
//! on `$PATH`. On this machine that is a few thousand candidates, and the whole point of
//! [`Fuzzed`] is that the pattern is folded once for the batch instead of once per candidate.
//!
//! Run with `cargo bench --bench fuzzy` (release, so the numbers mean something).

use oslo::interactive::matching::{Fuzzed, Fuzzy, fuzzy_score};
use std::time::Instant;

/// Roughly what a `$PATH` looks like: many names, most of them not matching.
fn candidates(n: usize) -> Vec<String> {
    let stems = [
        "git",
        "cargo",
        "rustc",
        "systemctl",
        "kubectl",
        "python3",
        "grep",
        "awk",
        "sed",
        "find",
        "docker",
        "nix",
        "ssh",
        "curl",
        "tar",
        "gzip",
        "make",
        "gcc",
        "clang",
        "ld",
    ];
    (0..n)
        .map(|i| format!("{}-{i}", stems[i % stems.len()]))
        .collect()
}

fn main() {
    let names = candidates(3300);
    let typed = "gco";
    let rounds = 50;

    let start = Instant::now();
    let mut hits = 0usize;
    for _ in 0..rounds {
        for name in &names {
            if fuzzy_score(name, typed, Fuzzy::Smart).is_some() {
                hits += 1;
            }
        }
    }
    let per_call = start.elapsed() / rounds as u32;

    let start = Instant::now();
    let mut hoisted_hits = 0usize;
    for _ in 0..rounds {
        let pattern = Fuzzed::new(typed, Fuzzy::Smart);
        for name in &names {
            if pattern.score(name).is_some() {
                hoisted_hits += 1;
            }
        }
    }
    let per_batch = start.elapsed() / rounds as u32;

    assert_eq!(
        hits, hoisted_hits,
        "the two forms must agree on what matched"
    );
    println!("candidates per press : {}", names.len());
    println!("fold per candidate   : {per_call:?}");
    println!("fold once per press  : {per_batch:?}");
    let saved = per_call.as_secs_f64() - per_batch.as_secs_f64();
    println!(
        "saved                : {:.0} us ({:.0}%)",
        saved * 1e6,
        saved / per_call.as_secs_f64() * 100.0
    );
}
