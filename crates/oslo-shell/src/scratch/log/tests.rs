use super::*;

fn at(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("alpha.log")
}

#[test]
fn what_goes_in_comes_out() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = at(&dir);
    let mut log = Log::open(&path, 1024).expect("open");
    log.append(b"hello ").expect("append");
    log.append(b"world").expect("append");
    assert_eq!(std::fs::read(&path).expect("read"), b"hello world");
}

/// Reopening appends rather than starting again — a scratch outlives the process that last wrote to it.
#[test]
fn reopening_keeps_what_was_there() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = at(&dir);
    Log::open(&path, 1024)
        .expect("open")
        .append(b"first ")
        .expect("append");
    Log::open(&path, 1024)
        .expect("open")
        .append(b"second")
        .expect("append");
    assert_eq!(std::fs::read(&path).expect("read"), b"first second");
}

/// **The cap is the point.** /tmp is memory, so a chatty job must not be able to spend it all.
#[test]
fn it_stays_under_the_cap_and_keeps_the_newest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = at(&dir);
    let mut log = Log::open(&path, 100).expect("open");
    for n in 0..100 {
        log.append(format!("line {n}\n").as_bytes())
            .expect("append");
    }
    let out = std::fs::read(&path).expect("read");
    assert!(out.len() <= 100, "cap not honoured: {} bytes", out.len());
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.contains("line 99"),
        "the newest output must survive: {text:?}"
    );
    assert!(!text.contains("line 0\n"), "the oldest must not: {text:?}");
}

/// A single write bigger than the whole cap keeps its tail rather than nothing.
#[test]
fn one_oversized_write_keeps_its_tail() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = at(&dir);
    let mut log = Log::open(&path, 10).expect("open");
    log.append(b"0123456789abcdef").expect("append");
    let out = std::fs::read(&path).expect("read");
    assert!(out.len() <= 10, "{} bytes", out.len());
    assert!(out.ends_with(b"f"), "the end of the write is what is kept");
}

/// The file keeps its identity across a trim, so a `tail -f` already running keeps following it.
#[test]
fn trimming_does_not_replace_the_file() {
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = at(&dir);
    let mut log = Log::open(&path, 50).expect("open");
    log.append(b"x").expect("append");
    let before = std::fs::metadata(&path).expect("stat").ino();
    for _ in 0..50 {
        log.append(b"yyyyy").expect("append");
    }
    let after = std::fs::metadata(&path).expect("stat").ino();
    assert_eq!(before, after, "the inode changed, so tail -f would be lost");
}

/// Zero means no cap, for anyone who would rather have the whole thing.
#[test]
fn a_zero_cap_never_trims() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = at(&dir);
    let mut log = Log::open(&path, 0).expect("open");
    for _ in 0..200 {
        log.append(b"0123456789").expect("append");
    }
    assert_eq!(std::fs::read(&path).expect("read").len(), 2000);
}

#[test]
fn the_tail_is_what_is_replayed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = at(&dir);
    Log::open(&path, 0)
        .expect("open")
        .append(b"abcdefghij")
        .expect("append");
    assert_eq!(tail(&path, 4).expect("tail"), b"ghij");
    // Asking for more than there is answers everything, rather than failing.
    assert_eq!(tail(&path, 100).expect("tail"), b"abcdefghij");
}
