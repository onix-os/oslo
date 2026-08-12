//! Who is asking: this shell, and this machine.
//!
//! Both are recorded on every run so history can be filtered by them. Neither can be worked out
//! afterwards from a line and a timestamp, which is the whole reason they are stored rather than
//! inferred: "did I run this in *this* shell" is a fact about the moment it ran.

use std::sync::OnceLock;

/// This shell's identity, stable for its lifetime and different from every other shell's.
///
/// The pid and the start time together, because a pid alone is reused — a shell started an hour
/// after one that died could otherwise inherit its history.
pub fn id() -> String {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        // **`$OSLO_SESSION` first.** A shell exports it, so everything the shell starts — a
        // subshell, a tool, `oslo macros` — names the session it is part of rather than inventing
        // one of its own. Without this a child process could not talk about the session it is in:
        // it would compute an id nobody else has ever heard of.
        if let Ok(named) = std::env::var("OSLO_SESSION")
            && !named.trim().is_empty()
        {
            return named;
        }
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("{}-{started}", std::process::id())
    })
    .clone()
}

/// This machine's short name — everything before the first dot.
///
/// Cached: it is read on every command that gets recorded, and it does not change while a shell
/// is running.
pub fn host() -> String {
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(|| {
        nix::unistd::gethostname()
            .ok()
            .and_then(|name| name.into_string().ok())
            .map(|name| name.split('.').next().unwrap_or(&name).to_string())
            .unwrap_or_default()
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id is stable within a process, or two commands from one shell would look like two
    /// sessions.
    #[test]
    fn the_session_id_is_stable() {
        assert_eq!(id(), id());
        assert!(!id().is_empty());
    }

    /// It carries the pid *and* a start time, because pids are reused.
    #[test]
    fn the_session_id_is_more_than_a_pid() {
        let id = id();
        let (pid, started) = id.split_once('-').expect("pid-started");
        assert_eq!(pid, std::process::id().to_string());
        assert!(started.parse::<u64>().expect("a timestamp") > 1_600_000_000);
    }

    #[test]
    fn the_host_is_the_short_name() {
        assert!(!host().contains('.'), "{}", host());
    }
}
