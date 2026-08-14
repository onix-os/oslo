//! A directory of sealed files, as one stream of bytes.
//!
//! # Why not tar
//!
//! Because this carries exactly one thing: a flat directory of files whose names oslo already
//! validates and whose contents it never looks inside. Tar brings permissions, ownership, symlinks,
//! device nodes and path traversal — every one of them a way for a hostile far end to write
//! somewhere it should not, in exchange for nothing this needs.
//!
//! ```text
//! OSLOBUNDLE1
//! per file:  name length (u32) │ name │ body length (u64) │ body
//! ```
//!
//! **Names are checked on the way in, not trusted.** A name with a `/` or a `..` in it is refused
//! rather than sanitised: the far end is another oslo and has no business sending one, so the
//! honest answer to a name that could escape the directory is to stop.

use std::path::Path;

const MAGIC: &[u8] = b"OSLOBUNDLE1\n";

/// Every file in `directory`, as one stream. A directory that is not there packs as empty.
pub fn pack(directory: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = MAGIC.to_vec();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Ok(bytes);
    };
    let mut files: Vec<(String, std::path::PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .collect();
    // Sorted so that the same directory always packs to the same bytes, which makes a difference
    // something a person can diff rather than something that moves on its own.
    files.sort();

    for (name, path) in files {
        // The scratch files a write leaves behind belong to whoever is writing, not in a bundle.
        if name.ends_with(".new") {
            continue;
        }
        let body = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let length = u32::try_from(name.len()).map_err(|_| format!("{name}: name too long"))?;
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&body);
    }
    Ok(bytes)
}

/// The other way, into a directory of its own.
pub fn unpack(directory: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    if bytes.is_empty() {
        return Ok(());
    }
    let mut rest = bytes
        .strip_prefix(MAGIC)
        .ok_or("what arrived is not a bundle of secrets")?;

    while !rest.is_empty() {
        let (length, tail) = take(rest, 4)?;
        let length = u32::from_le_bytes(length.try_into().map_err(|_| bad())?) as usize;
        let (name, tail) = take(tail, length)?;
        let name = std::str::from_utf8(name).map_err(|_| "a name in the bundle is not text")?;
        let (size, tail) = take(tail, 8)?;
        let size = u64::from_le_bytes(size.try_into().map_err(|_| bad())?) as usize;
        let (body, tail) = take(tail, size)?;
        rest = tail;

        if !safe(name) {
            return Err(format!("{name:?}: not a name a bundle may carry"));
        }
        let path = directory.join(name);
        std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// A name that can only land inside the directory it is unpacked into.
fn safe(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && name != "."
        && name != ".."
}

fn take(bytes: &[u8], how_many: usize) -> Result<(&[u8], &[u8]), String> {
    if bytes.len() < how_many {
        return Err(bad());
    }
    Ok(bytes.split_at(how_many))
}

fn bad() -> String {
    "the bundle of secrets is truncated or damaged".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(at: &Path, name: &str, body: &[u8]) {
        std::fs::create_dir_all(at).expect("dir");
        std::fs::write(at.join(name), body).expect("write");
    }

    #[test]
    fn a_directory_survives_the_round_trip() {
        let dir = tempfile::tempdir().expect("dir");
        let from = dir.path().join("from");
        write(&from, "one.sealed", b"first");
        write(&from, "two.sealed", &[0, 159, 146, 150]);

        let into = dir.path().join("into");
        unpack(&into, &pack(&from).expect("pack")).expect("unpack");
        assert_eq!(
            std::fs::read(into.join("one.sealed")).expect("read"),
            b"first"
        );
        assert_eq!(
            std::fs::read(into.join("two.sealed")).expect("read"),
            [0, 159, 146, 150]
        );
    }

    /// The same directory always packs to the same bytes.
    #[test]
    fn packing_is_stable() {
        let dir = tempfile::tempdir().expect("dir");
        let from = dir.path().join("from");
        write(&from, "b.sealed", b"two");
        write(&from, "a.sealed", b"one");
        assert_eq!(pack(&from).expect("pack"), pack(&from).expect("pack"));
    }

    #[test]
    fn an_empty_or_missing_directory_is_not_an_error() {
        let dir = tempfile::tempdir().expect("dir");
        let packed = pack(&dir.path().join("nothing-here")).expect("pack");
        let into = dir.path().join("into");
        unpack(&into, &packed).expect("unpack");
        assert_eq!(std::fs::read_dir(&into).expect("read").count(), 0);
    }

    /// **A hostile far end cannot write outside the directory.**
    #[test]
    fn a_name_that_would_escape_is_refused() {
        let dir = tempfile::tempdir().expect("dir");
        for bad in ["../escaped", "sub/dir", "..", "."] {
            let mut bytes = MAGIC.to_vec();
            bytes.extend_from_slice(&(bad.len() as u32).to_le_bytes());
            bytes.extend_from_slice(bad.as_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
            let into = dir.path().join("into");
            assert!(unpack(&into, &bytes).is_err(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn a_damaged_bundle_is_refused_rather_than_half_read() {
        let dir = tempfile::tempdir().expect("dir");
        let from = dir.path().join("from");
        write(&from, "one.sealed", b"first");
        let packed = pack(&from).expect("pack");

        let into = dir.path().join("into");
        assert!(unpack(&into, &packed[..packed.len() - 2]).is_err());
        assert!(unpack(&into, b"not a bundle at all").is_err());
    }

    /// A half-written file is somebody's in-progress write, not part of the store.
    #[test]
    fn scratch_files_are_left_behind() {
        let dir = tempfile::tempdir().expect("dir");
        let from = dir.path().join("from");
        write(&from, "one.sealed", b"first");
        write(&from, "one.new", b"half");

        let into = dir.path().join("into");
        unpack(&into, &pack(&from).expect("pack")).expect("unpack");
        assert!(into.join("one.sealed").exists());
        assert!(!into.join("one.new").exists());
    }
}
