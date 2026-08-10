//! What the predictor costs, on the two paths where cost decides whether it can exist.
//!
//! Both numbers here are gates rather than curiosities:
//!
//! * **Replay** is what building the model from history costs. oslo starts in about 3.5 ms and
//!   beats bash; if replaying a real history is slower than that, the model cannot be built at
//!   startup and must come from a snapshot or a thread.
//! * **Predict** is what one keystroke costs. The ghost suggestion path is measured in
//!   microseconds — `bench/keystroke.rs` puts the hint at ~2.3 µs — so a prediction that takes
//!   half a millisecond is not shippable however good its answers are.
//!
//! Run with `cargo bench --bench predict` (release, so the numbers mean something).

use oslo::track::log::Entry;
use std::time::Instant;

/// A history of `n` commands across a handful of shells, with the repetition real history has.
fn history(n: usize) -> Vec<Entry> {
    const LINES: [&str; 12] = [
        "cargo build",
        "cargo test",
        "git status",
        "git add -p",
        "git commit",
        "ls -la",
        "cd ..",
        "make verify",
        "rg todo",
        "vim src/main.rs",
        "cargo clippy --all-targets",
        "git push origin develop",
    ];
    (0..n)
        .map(|i| Entry {
            line: LINES[i % LINES.len()].to_string(),
            mode: "sh".to_string(),
            // Four shells, so streams are exercised rather than one long run.
            session: (i % 4) as u32 + 1,
            seq: (i / 4) as u32 + 1,
            rewritten: false,
        })
        .collect()
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a duration"));
    samples[samples.len() / 2]
}

fn main() {
    for size in [1_000usize, 10_000, 50_000] {
        let entries = history(size);

        let mut built = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            let mut model = oslo::predict::Model::new();
            model.learn_all(&entries);
            built.push(started.elapsed().as_secs_f64());
            std::hint::black_box(model.learned());
        }
        let replay = median(built);

        let mut model = oslo::predict::Model::new();
        model.learn_all(&entries);

        let mut bytes = Vec::new();
        let started = Instant::now();
        model.save(&mut bytes).expect("write");
        let save = started.elapsed().as_secs_f64();

        let mut loads = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            let read = oslo::predict::Model::load(&bytes[..]).expect("read");
            loads.push(started.elapsed().as_secs_f64());
            std::hint::black_box(read.learned());
        }
        let load = median(loads);

        // One prediction, the way the ghost asks for it: a partial line and a handful of answers.
        let mut asked = Vec::new();
        for _ in 0..7 {
            let started = Instant::now();
            for _ in 0..1_000 {
                std::hint::black_box(model.next(1, 500, Some("car"), 3));
            }
            asked.push(started.elapsed().as_secs_f64() / 1_000.0);
        }
        let predict = median(asked);

        // And one repair, which is not on the keystroke path but must not feel like a pause.
        let mut repairs = Vec::new();
        for _ in 0..7 {
            let started = Instant::now();
            for _ in 0..100 {
                std::hint::black_box(model.repair(1, 500, "carg buld", 3));
            }
            repairs.push(started.elapsed().as_secs_f64() / 100.0);
        }
        let repair = median(repairs);

        println!(
            "{size:>6} commands   replay {:>8.1} ms   snapshot {:>6.1} KB  save {:>6.1} ms  load {:>6.1} ms   predict {:>7.1} us   repair {:>7.1} us",
            replay * 1e3,
            bytes.len() as f64 / 1024.0,
            save * 1e3,
            load * 1e3,
            predict * 1e6,
            repair * 1e6,
        );
    }
}
