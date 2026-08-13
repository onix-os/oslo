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
        // **`$OSLO_SESSION` first, for anything that is not itself a shell.** A tool oslo starts —
        // `oslo macros`, a hook, a subshell running a helper — has to be able to name the session
        // it is part of, and without this it would compute an id nobody has ever heard of and turn
        // a macro off for a session nobody is running.
        //
        // A **shell** does not take the inherited one: it calls [`begin`] first, which stamps a new
        // id over it. A nested shell is its own session — the terminal integration marks every
        // command with it, and two shells sharing one id would be two shells the terminal cannot
        // tell apart.
        if let Ok(named) = std::env::var("OSLO_SESSION")
            && !named.trim().is_empty()
        {
            return named;
        }
        fresh()
    })
    .clone()
}

/// Start a new session, and export it.
///
/// Called by a process that *is* a shell, before anything reads [`id`]. Everything it starts
/// inherits `$OSLO_SESSION` and so agrees with it about which session this is.
pub fn begin() -> String {
    let id = fresh();
    // SAFETY: called once, from `main`, before any thread is started.
    unsafe { std::env::set_var("OSLO_SESSION", &id) };
    id
}

/// An id nothing else will have: this process, and when it started.
fn fresh() -> String {
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}-{started}", std::process::id())
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
    ///
    /// Asked of [`fresh`] rather than of [`id`]: `id` answers with the *inherited* `$OSLO_SESSION`
    /// when there is one, and a test run from an oslo shell has one — so asserting that `id` names
    /// this process would be asserting that nothing started this test, which is false and was how
    /// this test failed the moment a session could be inherited at all.
    #[test]
    fn the_session_id_is_more_than_a_pid() {
        let id = fresh();
        let (pid, started) = id.split_once('-').expect("pid-started");
        assert_eq!(pid, std::process::id().to_string());
        assert!(started.parse::<u64>().expect("a timestamp") > 1_600_000_000);
    }

    /// **Two shells are two sessions; a tool is not a shell.** `id` takes the inherited
    /// `$OSLO_SESSION` so that `oslo macros off --session` names the shell that started it, and
    /// `begin` stamps a new one so a nested shell is its own — the terminal marks every command
    /// with it, and two shells sharing an id would be two the terminal cannot tell apart.
    #[test]
    fn a_shell_starts_a_session_and_a_tool_joins_one() {
        // `fresh` is what both are built from, and no two calls agree.
        let one = fresh();
        assert!(one.contains('-'), "pid and start time: {one}");
        assert_eq!(one, fresh(), "same process, same second");

        // What a tool would read, without touching the process environment every other test shares.
        let inherited = "424242-1700000000";
        assert_ne!(inherited, fresh(), "a tool's own id is not the shell's");
    }

    #[test]
    fn the_host_is_the_short_name() {
        assert!(!host().contains('.'), "{}", host());
    }
}
