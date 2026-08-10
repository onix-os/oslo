//! What one keystroke costs, measured rather than reasoned about.
//!
//! The editor repaints the whole line on every key, and everything the repaint consults is asked
//! again each time: the theme, the settings, and — for colouring — whether each word is a builtin,
//! a function or an alias. None of that has a harness anywhere else, so a change to the typing
//! path could only ever be argued for on paper.
//!
//! Two things are measured, both per keystroke rather than per line, because that is the unit a
//! person feels:
//!
//! * **paint** — syntax colouring of the line so far, which is what a repaint spends its time on.
//! * **settings** — reading the config the editor consults two to four times per key.
//!
//! There is no terminal here and no input: painting a line is a pure function of the line and the
//! environment, which is exactly why it can be measured this way.
//!
//! Run with `cargo bench --bench keystroke` (release, so the numbers mean something).

use oslo::ui::OsloHelper;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A line typed one character at a time, as the editor sees it: `t`, `ta`, `tar`, …
fn prefixes(line: &str) -> Vec<String> {
    (1..=line.len()).map(|n| line[..n].to_string()).collect()
}

fn helper() -> OsloHelper {
    OsloHelper::new(Arc::new(Mutex::new(oslo::Environment::new())))
}

/// Median of `runs`, which is what to report when the machine is not quiet.
fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a duration"));
    samples[samples.len() / 2]
}

fn bench_paint(helper: &OsloHelper) {
    // A realistic line: a builtin, an external, a path, a flag and a redirection — every branch
    // the colourer has, and every word it has to classify.
    let line = "export PATH=/usr/bin && cargo build --release 2> /tmp/log";
    let keys = prefixes(line);

    let mut samples = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        for key in &keys {
            std::hint::black_box(helper.paint(key));
        }
        samples.push(started.elapsed().as_secs_f64() / keys.len() as f64);
    }
    println!(
        "paint      {:>8.2} us/keystroke   ({} keys, line of {} chars)",
        median(samples) * 1e6,
        keys.len(),
        line.len()
    );
}

/// The ghost suggestion, which is the other thing every keystroke asks for.
///
/// A command word rather than a path, so the `$PATH` index is what answers — a few thousand names
/// on a normal machine, and the reason this is worth a benchmark at all.
fn bench_hint(helper: &OsloHelper) {
    let keys = prefixes("cargo");

    let mut samples = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        for _ in 0..200 {
            for key in &keys {
                std::hint::black_box(helper.command_hint(key, key.len()));
            }
        }
        samples.push(started.elapsed().as_secs_f64() / (200 * keys.len()) as f64);
    }
    println!(
        "hint       {:>8.2} us/keystroke   (command word, against all of $PATH)",
        median(samples) * 1e6
    );
}

/// The repair hint, which every keystroke asks for **when there is no suggestion to draw**.
///
/// Both cases, because the gap between them is the design. A word that is a *prefix* of something
/// runnable is answered by a binary search and costs nothing; only a word that is neither a command
/// nor the start of one reaches the edit distance over every name on `$PATH`. Measure only the
/// second and the feature looks fifteen times more expensive than the repaint it sits in; measure
/// only the first and the worst case is hidden.
fn bench_repair(helper: &OsloHelper) {
    for (line, what) in [
        ("cargo build --release", "a real command, the ordinary case"),
        ("lsvlk", "a misspelling, edit distance over all of $PATH"),
    ] {
        let keys = prefixes(line);
        let mut samples = Vec::new();
        for _ in 0..7 {
            let started = Instant::now();
            for _ in 0..200 {
                for key in &keys {
                    std::hint::black_box(helper.repair(key));
                }
            }
            samples.push(started.elapsed().as_secs_f64() / (200 * keys.len()) as f64);
        }
        println!(
            "repair     {:>8.2} us/keystroke   ({what})",
            median(samples) * 1e6
        );
    }
}

fn bench_settings() {
    let mut samples = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        for _ in 0..10_000 {
            std::hint::black_box(oslo::ui::settings::current());
        }
        samples.push(started.elapsed().as_secs_f64() / 10_000.0);
    }
    println!(
        "settings   {:>8.2} us/read        (the editor reads it 2-4 times per key)",
        median(samples) * 1e6
    );
}

fn main() {
    let helper = helper();
    // Once before measuring: the first paint resolves the theme and the colour depth, and that
    // one-off would otherwise land in the first sample.
    std::hint::black_box(helper.paint("cargo"));

    bench_paint(&helper);
    bench_hint(&helper);
    bench_repair(&helper);
    bench_settings();
}
