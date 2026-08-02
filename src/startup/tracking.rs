//! What the interactive loop already knows, handed to the store instead of dropped on the floor.
//!
//! One turn of [`super::repl`] computes the directory the command started in, the moment it
//! started, the directory it left the shell in, how long it took and what it exited with — all
//! within sixty lines — and then keeps two of them. Nothing here is new telemetry; it is those
//! same locals reaching a table. The whole of it is one call at the end of the loop.
//!
//! # Why not the `postcmd` hook
//!
//! Because that hook fires only on `Ok(status)`, so every command that failed outright would go
//! unrecorded — and failures are exactly what the `fails` column exists to hold. This runs beside
//! the hook, not through it.
//!
//! # Why the store is opened here and nowhere else
//!
//! [`crate::startup`] is the binary. A script, an `oslo -c` or a subshell never reaches this file,
//! so `track::store()` answers `None` for them and there is no file for a CI job's command lines to
//! land in — a structural answer rather than a flag somebody has to remember to check.

use super::history;
use oslo::error::ShellError;
use oslo::track::{self, Run, Step, Track, Visit};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Whether this session keeps any record of itself at all.
///
/// `HISTFILE=""` and `HISTSIZE=0` are already documented as the way to run a session that leaves no
/// trace. A user who took that step and then found a new tracking file had appeared beside the
/// history they had just switched off would be right to be angry — so such a session is given no
/// store rather than a quieter one. Read off [`history::Settings`] rather than off `$HISTFILE`
/// again, so the tracker cannot come to a different conclusion from the history about what was
/// asked for.
fn keeps_a_record(settings: &history::Settings) -> bool {
    settings.file.is_some() && settings.max_size > 0
}

/// The two things the loop cannot re-derive after the fact: when the shell arrived where it is
/// standing, and which worktree that is.
pub(super) struct Tracker {
    /// The open dwell segment. It lives here rather than in a row with a null end, because a row
    /// with a null end is precisely the thing that cannot survive a `kill -9`; only closed
    /// segments are ever written.
    ///
    /// Wall clock rather than `Instant`, deliberately: a laptop that slept for nine hours in `~`
    /// really did spend them there, and it is the cap the store applies, not a monotonic clock,
    /// that stops those hours from counting as nine hours of interest.
    since: SystemTime,
    /// The last directory a worktree was walked for. A one-entry cache is the whole of it: a shell
    /// stands still for long stretches, and the directory a command ran in is nearly always the
    /// one the command before it left behind.
    worktree: Option<(String, Option<String>)>,
}

impl Tracker {
    /// Open the store, hand it to the process, and tell it where the shell is standing.
    pub(super) fn start(here: &str, settings: &history::Settings) -> Tracker {
        let mut tracker = Tracker {
            since: SystemTime::now(),
            worktree: None,
        };
        if !keeps_a_record(settings) {
            return tracker;
        }
        track::install(
            track::default_path(
                std::env::var("XDG_DATA_HOME").ok().as_deref(),
                std::env::var("HOME").ok().as_deref(),
            )
            .and_then(|path| Track::open(&path)),
        );
        // Priming rather than counting a visit: starting a shell somewhere is not the same act as
        // walking there, and it must not raise that directory's rank. What it buys is a `dir_id`
        // for the session's first command, which the visit statement would not have written
        // because the directory never changed.
        if let Some(track) = track::store() {
            let root = tracker.worktree_of(here);
            track.prime(&Visit {
                path: here,
                root: root.as_deref(),
            });
        }
        tracker
    }

    /// Write down one turn of the loop.
    pub(super) fn boundary(&mut self, before: &str, after: &str, run: Option<Run<'_>>) {
        let Some(track) = track::store() else {
            return;
        };
        self.write(track, before, after, run);
    }

    /// A line the user asked the shell to forget as they typed it.
    ///
    /// The segment is closed and thrown away, so the time a secret command took is not credited to
    /// the directory it ran in either. The leading space is the one privacy mechanism a user
    /// operates deliberately; half-honouring it — dropping the line but keeping the place and the
    /// minutes — would be worse than not offering it.
    pub(super) fn forget_boundary(&mut self) {
        self.since = SystemTime::now();
    }

    /// The statements, against a store the caller has already found.
    ///
    /// Separate from [`Tracker::boundary`] so that it can be tested against a store in a temporary
    /// directory rather than against the process-global one, which can only ever be set once.
    fn write(&mut self, track: &Track, before: &str, after: &str, run: Option<Run<'_>>) {
        let dwell = self.close_segment();
        let moved = after != before;
        let here = self.worktree_of(before);
        let there = if moved { self.worktree_of(after) } else { None };
        track.record(&Step {
            ran_in: Visit {
                path: before,
                root: here.as_deref(),
            },
            moved_to: moved.then_some(Visit {
                path: after,
                root: there.as_deref(),
            }),
            dwell_ms: dwell,
            run,
        });
    }

    /// Close the open dwell segment and start the next one.
    ///
    /// A clock that went backwards contributes nothing rather than taking time away from a
    /// directory that had earned it.
    fn close_segment(&mut self) -> i64 {
        let now = SystemTime::now();
        let elapsed = now.duration_since(self.since).unwrap_or(Duration::ZERO);
        self.since = now;
        millis(elapsed)
    }

    /// The worktree `dir` belongs to, walked once per directory rather than once per command.
    fn worktree_of(&mut self, dir: &str) -> Option<String> {
        if let Some((cached, root)) = &self.worktree
            && cached == dir
        {
            return root.clone();
        }
        let root = oslo::interactive::prompt::git_root_of(Path::new(dir))
            .map(|root| root.to_string_lossy().into_owned());
        self.worktree = Some((dir.to_string(), root.clone()));
        root
    }
}

/// The command as the store wants it, or `None` when there is nothing worth writing down.
///
/// A line that failed to *parse* is deliberately not a command. A typo is often a password typed
/// into the wrong prompt, and a store built to suggest lines back to you is the last place one
/// should come to rest.
pub(super) fn ran<'a>(
    text: &'a str,
    mode: &'a str,
    result: &Result<i32, ShellError>,
    elapsed: Duration,
) -> Option<Run<'a>> {
    let status = match result {
        Ok(status) => *status,
        Err(ShellError::SyntaxError(_)) => return None,
        // `exit 0` succeeded; every other error is the failure `fails` is counting.
        Err(ShellError::Exit(code)) => *code,
        Err(err) => err.failure_status(),
    };
    Some(Run {
        argv: text,
        mode,
        status: Some(status),
        duration_ms: millis(elapsed),
    })
}

/// A duration in whole milliseconds. Sub-millisecond resolution on a shell command is noise.
fn millis(elapsed: Duration) -> i64 {
    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SH: &str = "sh";

    fn store() -> (tempfile::TempDir, Track) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let track = Track::open(&dir.path().join("track.db")).expect("the database opens");
        (dir, track)
    }

    fn tracker() -> Tracker {
        Tracker {
            since: SystemTime::now(),
            worktree: None,
        }
    }

    /// The whole point of the write path, asserted through the read side: the same prefix means a
    /// different line depending on where it is typed.
    #[test]
    fn a_command_is_attributed_to_where_it_started_not_where_it_left_you() {
        let (_dir, track) = store();
        let mut tracker = tracker();

        // `cd /w/beta` ran in /w and left the shell in /w/beta.
        tracker.write(
            &track,
            "/w",
            "/w/beta",
            ran("cd /w/beta", SH, &Ok(0), Duration::from_millis(1)),
        );
        tracker.write(
            &track,
            "/w/beta",
            "/w/beta",
            ran(
                "cargo run --example abc",
                SH,
                &Ok(0),
                Duration::from_millis(20),
            ),
        );

        assert_eq!(
            track.suggestion_here("/w/beta", SH, "cargo run --ex"),
            Some("cargo run --example abc".to_string())
        );
        assert_eq!(
            track.suggestion_here("/w", SH, "cargo run --ex"),
            None,
            "the cd was attributed to /w; what it ran into was not"
        );
        assert_eq!(
            track.suggestion_here("/w", SH, "cd /w/b"),
            Some("cd /w/beta".to_string())
        );
    }

    /// A command that never finished parsing is not a command, and a command that failed is
    /// recorded as having failed rather than not recorded at all.
    #[test]
    fn what_reaches_the_store_and_what_does_not() {
        let syntax = Err(ShellError::SyntaxError("unexpected token".to_string()));
        assert!(ran("mypassword )", SH, &syntax, Duration::ZERO).is_none());

        let failed = ran("cargo buidl", SH, &Ok(101), Duration::from_millis(3));
        assert_eq!(failed.map(|run| run.status), Some(Some(101)));

        // `exit 0` succeeded, however the loop learned about it.
        let quit = ran("exit", SH, &Err(ShellError::Exit(0)), Duration::ZERO);
        assert_eq!(quit.map(|run| run.status), Some(Some(0)));
    }

    /// A session that was told to leave no trace leaves none of this either.
    #[test]
    fn a_session_that_keeps_no_history_opens_no_store() {
        let kept = |file: Option<&str>, max_size| {
            keeps_a_record(&history::Settings {
                file: file.map(std::path::PathBuf::from),
                max_size,
            })
        };
        assert!(kept(Some("/home/u/.oslo_history"), 10_000));
        assert!(!kept(None, 10_000), "HISTFILE= disables the store with it");
        assert!(
            !kept(Some("/home/u/.oslo_history"), 0),
            "and so does HISTSIZE=0"
        );
    }

    /// A secret line leaves nothing behind — not the line, not the directory, not the minutes.
    #[test]
    fn a_secret_line_is_not_a_boundary_at_all() {
        let (_dir, track) = store();
        let mut tracker = tracker();
        tracker.forget_boundary();

        // Nothing was written, so the directory the secret command ran in is not even known.
        assert_eq!(track.suggestion_here("/w/alpha", SH, "pass"), None);
        assert!(track.directories_named("alpha", "/w", 10).is_empty());

        // The next ordinary command still records normally.
        tracker.write(
            &track,
            "/w",
            "/w/alpha",
            ran("cd alpha", SH, &Ok(0), Duration::ZERO),
        );
        assert_eq!(
            track
                .directories_named("alpha", "/w", 10)
                .into_iter()
                .map(|found| found.path)
                .collect::<Vec<_>>(),
            vec!["/w/alpha".to_string()]
        );
    }
}
