//! What a damaged database does, which is answer rather than die.

use super::super::*;
use super::run_key;

/// **A corrupt database is reported, never a panic**, and that is what lets the release build be
/// compiled with `panic = "abort"`.
///
/// There used to be a test here asserting that a write which *panicked* left the store usable. It
/// tested a `catch_unwind` that wrapped every read and write, and that guard existed for one
/// reason: tagdata could panic on a page it had read off disk. tagdata v0.1.4 validates a page
/// pointer before following it and answers `InvalidDB` instead, so the guards are gone — and with
/// them the 820 KB of unwinding tables the whole binary was carrying.
///
/// This is the property that replaced it. Bytes are scribbled through a real database and the
/// store is asked to read: the answer may be anything at all, including nothing, but it must be an
/// *answer*.
#[test]
fn a_corrupt_database_is_reported_rather_than_fatal() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("nested/track.kv");
    let key = run_key(1, "sh", "cargo build");
    {
        let store = Store::open(&path).expect("the store opens");
        for n in 0..64 {
            let filled = run_key(n, "sh", "a command with enough text to fill some pages");
            store.write(|w| w.put(Tree::Run, filled, b"value".to_vec()));
        }
        store.write(|w| w.put(Tree::Run, key.clone(), b"kept".to_vec()));
    }

    // Every byte past the meta pages, walked in steps that are not a factor of the page size, so
    // the damage lands in headers, keys and values alike rather than in one tidy place.
    let mut bytes = std::fs::read(&path).expect("the file reads");
    let mut seed = 0x9e37_79b9_u32;
    for at in (8192..bytes.len()).step_by(97) {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        bytes[at] = (seed >> 24) as u8;
    }
    std::fs::write(&path, &bytes).expect("the file writes");

    // Opening may fail, which is a fine answer. What it may not do is take the process with it.
    if let Some(store) = Store::open(&path) {
        let _: Option<bool> = store.read(|r| Some(r.has(Tree::Run, &key)));
        let _: Option<Vec<u8>> = store.read(|r| r.get(Tree::Run, &key));
        let _: Option<usize> = store.read(|r| Some(r.count(Tree::Run, &Span::all())));
    }
}
