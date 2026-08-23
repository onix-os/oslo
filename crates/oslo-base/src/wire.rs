//! Where a tool's control socket lives, and how a message on it is framed.
//!
//! One module because the two facts have exactly one job between them: letting two processes that
//! were written separately meet. A client that computes the path differently from the server finds
//! nothing; a reader that frames differently from the writer desynchronises on the first message.
//! Both belong in one place that each side reads rather than restates.
//!
//! # The path
//!
//! ```text
//! $XDG_RUNTIME_DIR/onix/<tool>/<session>.sock
//! ```
//!
//! `$XDG_RUNTIME_DIR` because it is per-user, mode 0700 by the login manager, and cleared when the
//! session ends — so a socket left behind by a killed shell does not outlive the login, and no other
//! user can reach it. `/tmp` has none of those properties and is the reason a world-readable
//! `/tmp/hexe-env-*` was a finding rather than a design.
//!
//! Falling back to `/tmp/onix-<uid>` when there is no runtime directory keeps a `su -` session or a
//! container working, and the uid in the name keeps two users off each other's path.
//!
//! # The frame
//!
//! Four bytes of big-endian length, then that many bytes of body. A stream has no message
//! boundaries, and every reader that does without a length prefix ends up inventing a delimiter and
//! then escaping it.

use std::path::PathBuf;

/// The directory every tool in the family puts its sockets under.
pub const FAMILY: &str = "onix";

/// Bytes of length prefix on every frame.
pub const HEADER: usize = 4;

/// What `sockaddr_un.path` holds, including the terminator.
///
/// A hard kernel limit, and low enough to reach by accident: a long `$XDG_RUNTIME_DIR`, a session
/// id and a `.sock` add up. `bind` truncates silently at 108 and then fails with "address already
/// in use" pointing at a path that is not the one asked for, which is a bad half-hour for whoever
/// meets it. [`too_long`] is the check that turns it into a sentence.
pub const MAX_SOCKET_PATH: usize = 108;

/// The largest body either side will read or write.
///
/// A control socket serves occasional questions, not traffic: a request is a small table of
/// arguments and an answer is a list of flat records. This is the ceiling that stops one
/// pathological call allocating without end, on both sides of the connection.
pub const MAX_FRAME: usize = 4 * 1024 * 1024;

/// `$XDG_RUNTIME_DIR/onix`, or a per-user directory under `/tmp` when there is none.
pub fn family_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        Some(runtime) => PathBuf::from(runtime).join(FAMILY),
        None => PathBuf::from(format!("/tmp/{FAMILY}-{}", nix::unistd::getuid().as_raw())),
    }
}

/// Where `tool`'s socket for `session` is, or would be.
///
/// Answers a path whether or not anything is listening: finding out is the caller's `connect`, and a
/// function that stat'd here would answer differently depending on when it was asked.
///
/// `None` for the session means this process's own, which is what a tool asks when it is about to
/// bind one.
pub fn socket_path(tool: &str, session: Option<&str>) -> PathBuf {
    let session = session
        .map(str::to_string)
        .unwrap_or_else(crate::track::session::id);
    family_dir().join(tool).join(format!("{session}.sock"))
}

/// Whether `path` is past what a unix socket address can hold — see [`MAX_SOCKET_PATH`].
///
/// Checked in bytes rather than characters, because that is what the kernel copies.
pub fn too_long(path: &std::path::Path) -> bool {
    path.as_os_str().len() >= MAX_SOCKET_PATH
}

/// Every session of `tool` that has a socket file, newest first.
///
/// The file existing does not mean anything is listening — a killed process leaves one behind. A
/// caller picks a candidate and connects; the failure *is* the staleness check, and it is the only
/// one that cannot be raced.
pub fn sessions_of(tool: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(family_dir().join(tool)) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|end| end == "sock"))
        .filter_map(|path| {
            let when = path.metadata().and_then(|m| m.modified()).ok()?;
            Some((when, path))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, path)| path).collect()
}

/// The four-byte header for a body of `len`.
pub fn header(len: usize) -> [u8; HEADER] {
    (len as u32).to_be_bytes()
}

/// The body length a header names, or `None` when it is over [`MAX_FRAME`].
///
/// Refused rather than clamped: a length that large is a desynchronised stream or a hostile peer,
/// and reading `MAX_FRAME` of whatever follows would be obeying it.
pub fn body_len(head: [u8; HEADER]) -> Option<usize> {
    let len = u32::from_be_bytes(head) as usize;
    (len <= MAX_FRAME).then_some(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path is per tool and per session, so two shells do not share a socket and two tools do
    /// not collide.
    #[test]
    fn a_socket_path_names_its_tool_and_session() {
        let one = socket_path("oslo", Some("session-one"));
        let two = socket_path("oslo", Some("session-two"));
        let other = socket_path("hexe", Some("session-one"));

        assert!(one.ends_with("oslo/session-one.sock"), "{one:?}");
        assert_ne!(one, two, "two sessions of one tool are two sockets");
        assert_ne!(one, other, "two tools are two sockets");
        assert_eq!(one.parent(), two.parent(), "and one directory per tool");
    }

    /// The address limit is checked, because `bind` truncates silently rather than refusing.
    #[test]
    fn an_over_long_socket_path_is_recognised() {
        let short = socket_path("oslo", Some("s"));
        assert!(!too_long(&short), "{short:?} should fit");

        let long = std::path::PathBuf::from("/tmp").join("x".repeat(MAX_SOCKET_PATH));
        assert!(
            too_long(&long),
            "{} bytes should not",
            long.as_os_str().len()
        );

        // The boundary itself: 108 is one too many, since the address includes a terminator.
        let edge = std::path::PathBuf::from("y".repeat(MAX_SOCKET_PATH - 1));
        assert!(!too_long(&edge));
        assert!(too_long(&std::path::PathBuf::from(
            "y".repeat(MAX_SOCKET_PATH)
        )));
    }

    /// A header round-trips, and one naming more than the ceiling is refused rather than clamped.
    #[test]
    fn a_frame_header_round_trips_and_a_huge_one_is_refused() {
        assert_eq!(body_len(header(0)), Some(0));
        assert_eq!(body_len(header(1)), Some(1));
        assert_eq!(body_len(header(MAX_FRAME)), Some(MAX_FRAME));
        assert_eq!(body_len((MAX_FRAME as u32 + 1).to_be_bytes()), None);
        assert_eq!(body_len(u32::MAX.to_be_bytes()), None);
    }
}
