//! The reads the write path pays for.
//!
//! Two questions, and the store answers both from an index. "Which remembered directory did that
//! keyword mean" is a range on `dir(base)`; "what did I run here that starts like this" is a range
//! on `run(dir_id, mode, argv)`.
//!
//! Neither is a `LIKE`. That is the single most performance-relevant decision in the store:
//! `argv LIKE 'cargo run --ex%'` finds the index by `dir_id` and then scans every row for that
//! directory, whereas `argv >= 'cargo run --ex' AND argv < 'cargo run --ey'` is a true B-tree range
//! scan — measured at 13 µs against 25,000 rows. Three orders of magnitude of headroom on a
//! per-keystroke budget is what makes a cache unnecessary, and therefore what makes a
//! cache-coherence problem between terminals impossible. deja needed a daemon largely to hold a
//! cache it then could not invalidate.
//!
//! The ordering of the directory reads is [`super::score::score_sql`] rather than a second
//! expression written out here, so that the ranking done in the database and the ranking done in
//! Rust cannot drift apart.

use super::db::{Track, now, runtime, upper_bound};
use super::score::{Candidate, score_sql};
use turso::{IntoParams, Value};

/// The best remembered line in one directory that starts with what has been typed.
///
/// `last_status = 0 OR runs > fails` is a defect this fixes as a side effect: today's suggestion
/// offers the newest prefix match with no idea whether it ever worked, so a typo is suggested for
/// ever. A null `last_status` is a command whose exit was never seen, and `= 0` correctly excludes
/// it rather than mistaking it for a success.
///
/// `argv <> ?3` drops the line already typed in full — a suggestion that adds nothing would be
/// drawn as empty ghost text. A *longer* line starting with it is still worth offering.
const SUGGEST_HERE: &str = "SELECT argv FROM run \
     WHERE dir_id = (SELECT id FROM dir WHERE path = ?1) \
       AND mode = ?2 AND argv >= ?3 AND argv < ?4 AND argv <> ?3 \
       AND (last_status = 0 OR runs > fails) \
     ORDER BY (runs - fails) DESC, last_at DESC LIMIT 1";

/// The same question widened to the whole worktree, for when you typed it upstairs.
///
/// You ran `cargo run --example abc` in the repository root and you are now in `crates/api`;
/// asking about the exact directory finds nothing. `dir.root` was written at visit time from the
/// caller's git-root walk, so there is no `git` subprocess anywhere near the keystroke path.
const SUGGEST_IN_WORKSPACE: &str = "SELECT r.argv FROM run r JOIN dir d ON d.id = r.dir_id \
     WHERE d.root = ?1 AND r.mode = ?2 AND r.argv >= ?3 AND r.argv < ?4 AND r.argv <> ?3 \
       AND (r.last_status = 0 OR r.runs > r.fails) \
     ORDER BY (r.runs - r.fails) DESC, r.last_at DESC LIMIT 1";

impl Track {
    /// Remembered directories whose final component starts with `needle`, best first.
    ///
    /// This is both indexed tiers at once — naming a directory exactly is also naming its prefix —
    /// as a half-open range on `dir_base`. Which of the two a row actually earned is decided in
    /// [`super::score`] against the path, so that the tier and the ordering are read off the same
    /// text in the same place.
    ///
    /// `exclude` is where the shell is standing. It is a bind rather than a comparison the caller
    /// makes afterwards, which is the one thing zoxide gets structurally wrong here: it excludes
    /// `$PWD` from its *shell function*, by string equality against `pwd`, and so silently fails
    /// whenever `pwd -L` and `pwd -P` disagree.
    pub fn directories_named(&self, needle: &str, exclude: &str, limit: usize) -> Vec<Candidate> {
        let needle = needle.to_lowercase();
        let Some(upper) = upper_bound(&needle) else {
            return Vec::new();
        };
        let sql = format!(
            "SELECT path, visits, last_visit FROM dir \
             WHERE base >= ?1 AND base < ?2 AND path <> ?3 AND path <> ?6 \
             ORDER BY {} DESC, length(path) ASC LIMIT ?5",
            score_sql("?4")
        );
        self.candidates(
            &sql,
            (
                needle,
                upper,
                exclude,
                now(),
                limit as i64,
                self.not_a_target(),
            ),
        )
    }

    /// The best-scoring remembered directories, for the tiers no index can serve.
    ///
    /// zoxide does this scan on every query; here it is only reached when naming the directory
    /// found nothing, and only for a `cd` a person typed. Measured at 1.07 ms over 3000
    /// directories, which is affordable exactly once per keypress of `Enter` and never per
    /// keystroke.
    pub fn directories_ranked(&self, exclude: &str, limit: usize) -> Vec<Candidate> {
        let sql = format!(
            "SELECT path, visits, last_visit FROM dir \
             WHERE path <> ?1 AND path <> ?4 \
             ORDER BY {} DESC, length(path) ASC LIMIT ?3",
            score_sql("?2")
        );
        self.candidates(&sql, (exclude, now(), limit as i64, self.not_a_target()))
    }

    /// The one remembered path that is never a jump destination, as a bind.
    ///
    /// `$HOME` is recorded like anywhere else — what you run there is worth suggesting — but `cd`
    /// with no operand already goes there, so offering it as a frecency candidate wins nothing.
    /// Empty when there is no `$HOME` to compare against, which no real path equals.
    fn not_a_target(&self) -> String {
        self.home
            .as_deref()
            .map(|home| home.trim_end_matches('/').to_string())
            .unwrap_or_default()
    }

    /// The line to suggest for `typed` in `dir`, or `None`.
    pub fn suggestion_here(&self, dir: &str, mode: &str, typed: &str) -> Option<String> {
        self.suggestion(SUGGEST_HERE, dir, mode, typed)
    }

    /// The line to suggest for `typed` anywhere in the worktree rooted at `root`, or `None`.
    pub fn suggestion_in_workspace(&self, root: &str, mode: &str, typed: &str) -> Option<String> {
        self.suggestion(SUGGEST_IN_WORKSPACE, root, mode, typed)
    }

    fn suggestion(&self, sql: &str, scope: &str, mode: &str, typed: &str) -> Option<String> {
        if typed.is_empty() {
            return None;
        }
        let upper = upper_bound(typed)?;
        runtime().block_on(async {
            let mut rows = self
                .conn
                .query(sql, (scope, mode, typed, upper))
                .await
                .ok()?;
            match rows.next().await {
                Ok(Some(row)) => match row.get_value(0) {
                    Ok(Value::Text(argv)) => Some(argv),
                    _ => None,
                },
                _ => None,
            }
        })
    }

    fn candidates(&self, sql: &str, params: impl IntoParams) -> Vec<Candidate> {
        runtime().block_on(async {
            let Ok(mut rows) = self.conn.query(sql, params).await else {
                return Vec::new();
            };
            let mut found = Vec::new();
            while let Ok(Some(row)) = rows.next().await {
                match (row.get_value(0), row.get_value(1), row.get_value(2)) {
                    (
                        Ok(Value::Text(path)),
                        Ok(Value::Integer(visits)),
                        Ok(Value::Integer(last_visit)),
                    ) => found.push(Candidate {
                        path,
                        visits,
                        last_visit,
                    }),
                    _ => break,
                }
            }
            found
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::db::fixture::*;
    use super::super::db::{Run, Step, Visit};

    /// Being worth remembering and being worth jumping to are different questions.
    ///
    /// The design excludes `$HOME` from the store outright (Privacy and size, item 6, agreeing with
    /// zoxide's default). Its stated reason — "your home directory is never a jump target" — is an
    /// argument about *candidates*, and applying it at write time also throws away every command
    /// run at home, silently killing directory-aware suggestion where many people spend the day.
    /// So the row is written and only the jump is refused.
    #[test]
    fn home_is_remembered_but_never_jumped_to() {
        let (_dir, track) = store_with_home("/home/u");
        track.record(&ran("/home/u", "cargo run --example home", 0));
        track.record(&ran("/home/u/src", "cargo run --example child", 0));

        assert_eq!(
            track.suggestion_here("/home/u", SH, "cargo run --ex"),
            Some("cargo run --example home".to_string()),
            "what you run at home is still suggested at home"
        );

        let ranked: Vec<String> = track
            .directories_ranked("/elsewhere", 10)
            .into_iter()
            .map(|found| found.path)
            .collect();
        assert!(
            !ranked.iter().any(|path| path == "/home/u"),
            "`cd` with no operand already goes home, so it is not a frecency candidate: {ranked:?}"
        );
        assert!(
            ranked.iter().any(|path| path == "/home/u/src"),
            "but its children are ordinary directories: {ranked:?}"
        );

        let named: Vec<String> = track
            .directories_named("u", "/elsewhere", 10)
            .into_iter()
            .map(|found| found.path)
            .collect();
        assert!(
            !named.iter().any(|path| path == "/home/u"),
            "and naming it does not reach it either: {named:?}"
        );
    }

    /// The same line in two directories is two rows, and that difference is the whole feature.
    #[test]
    fn the_directory_decides_which_line_is_offered() {
        let (_dir, track) = store();
        track.record(&ran("/w/alpha", "cargo run --example xyz", 0));
        track.record(&ran("/w/beta", "cargo run --example abc", 0));

        assert_eq!(count(&track, "SELECT COUNT(*) FROM run"), 2);
        assert_eq!(
            track.suggestion_here("/w/alpha", SH, "cargo run --ex"),
            Some("cargo run --example xyz".to_string())
        );
        assert_eq!(
            track.suggestion_here("/w/beta", SH, "cargo run --ex"),
            Some("cargo run --example abc".to_string())
        );
    }

    /// The half-open range must find every line that starts with what was typed and nothing that
    /// merely contains it — including at the boundary where the range ends.
    #[test]
    fn the_prefix_scan_finds_only_what_starts_with_it() {
        let (_dir, track) = store();
        for line in [
            "cargo test",
            "cargo tesseract",
            "cargo tf",
            "cargo tests --all",
            "make cargo test",
            "cargo t",
        ] {
            track.record(&ran("/w/alpha", line, 0));
        }

        // `cargo tf` sorts immediately above the range and `cargo t` immediately below it; `make
        // cargo test` contains the prefix but does not start with it.
        let offered = track.suggestion_here("/w/alpha", SH, "cargo te");
        assert!(
            matches!(
                offered.as_deref(),
                Some("cargo test" | "cargo tesseract" | "cargo tests --all")
            ),
            "got {offered:?}"
        );

        for _ in 0..3 {
            track.record(&ran("/w/alpha", "cargo test", 0));
        }
        assert_eq!(
            track.suggestion_here("/w/alpha", SH, "cargo te"),
            Some("cargo test".to_string()),
            "the one that has worked most often wins"
        );

        // A longer line is still worth offering; the line already typed in full never is.
        assert_eq!(
            track.suggestion_here("/w/alpha", SH, "cargo test"),
            Some("cargo tests --all".to_string())
        );
        assert_eq!(track.suggestion_here("/w/alpha", SH, "cargo tf"), None);

        // Nor does a suggestion cross languages, or reach a directory with nothing in it.
        assert_eq!(track.suggestion_here("/w/alpha", "lua", "cargo te"), None);
        assert_eq!(track.suggestion_here("/w/nowhere", SH, "cargo te"), None);
        assert_eq!(track.suggestion_here("/w/alpha", SH, ""), None);
    }

    /// A line that has only ever failed is not a suggestion — the defect this fixes in passing.
    #[test]
    fn a_command_that_never_worked_is_never_offered() {
        let (_dir, track) = store();
        track.record(&ran("/w/alpha", "carg build", 127));
        assert_eq!(track.suggestion_here("/w/alpha", SH, "carg"), None);

        // Once it has worked more often than not, it is a real command again.
        track.record(&ran("/w/alpha", "carg build", 0));
        track.record(&ran("/w/alpha", "carg build", 0));
        assert_eq!(
            track.suggestion_here("/w/alpha", SH, "carg"),
            Some("carg build".to_string())
        );
    }

    /// You typed it in the repository root and you are now three directories down.
    #[test]
    fn the_worktree_answers_when_the_exact_directory_cannot() {
        let (_dir, track) = store();
        let root = "/w/beta";
        assert!(track.record(&Step {
            ran_in: Visit {
                path: root,
                root: Some(root),
            },
            moved_to: None,
            dwell_ms: 0,
            run: Some(Run {
                argv: "cargo run --example abc",
                mode: SH,
                status: Some(0),
                duration_ms: 12,
            }),
        }));

        assert_eq!(
            track.suggestion_here("/w/beta/crates/api", SH, "cargo run"),
            None,
            "nothing was ever run down here"
        );
        assert_eq!(
            track.suggestion_in_workspace(root, SH, "cargo run"),
            Some("cargo run --example abc".to_string())
        );
        assert_eq!(
            track.suggestion_in_workspace("/w/alpha", SH, "cargo run"),
            None,
            "and it is this repository, not any repository"
        );
    }

    /// Naming a directory is an indexed range over the folded final component, and where you are
    /// standing is never among the answers.
    #[test]
    fn directories_are_found_by_the_name_they_end_in() {
        let (_dir, track) = store();
        for path in ["/w/Rust", "/w/rustlings", "/w/prust", "/w/go/src"] {
            track.record(&Step {
                ran_in: Visit::at("/w"),
                moved_to: Some(Visit::at(path)),
                dwell_ms: 0,
                run: None,
            });
        }

        let found: Vec<String> = track
            .directories_named("rust", "/w/nowhere", 10)
            .into_iter()
            .map(|candidate| candidate.path)
            .collect();
        assert_eq!(found.len(), 2, "got {found:?}");
        assert!(
            found.contains(&"/w/Rust".to_string()),
            "case is folded at write time"
        );
        assert!(
            found.contains(&"/w/rustlings".to_string()),
            "a prefix of the name counts"
        );
        assert!(
            !found.contains(&"/w/prust".to_string()),
            "but a substring of it does not — that is a weaker tier and a different query"
        );

        // Where you already are is never an answer, however well it matched.
        let found: Vec<String> = track
            .directories_named("rust", "/w/Rust", 10)
            .into_iter()
            .map(|candidate| candidate.path)
            .collect();
        assert_eq!(found, vec!["/w/rustlings".to_string()]);

        // The tiers no index can serve get every directory. `/w` is among them with nothing
        // counted: it is where the commands ran, so a row had to exist, but nobody walked there.
        let all = track.directories_ranked("/w/nowhere", 10);
        assert_eq!(all.len(), 5);
        assert!(all.iter().all(|candidate| {
            if candidate.path == "/w" {
                candidate.visits == 0
            } else {
                candidate.visits == 1 && candidate.last_visit > 0
            }
        }));

        // A limit cuts the list where the ranker would have cut it, not arbitrarily.
        assert_eq!(track.directories_ranked("/w/nowhere", 2).len(), 2);
    }
}
