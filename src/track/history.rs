//! Every command the shell remembers, aggregated for the history finder.
//!
//! The finder asks a different question from everything else in `query`. Those answer "what should
//! I suggest *here*" and are deliberately narrow — one directory, one prefix, the first good
//! answer. This answers "what have I ever run", because a full-screen finder filters in the
//! foreground as you type and cannot go back to the database on every keystroke.
//!
//! # Why the run table rather than the history file
//!
//! `startup::history_db` keeps lines in the order they were typed, which is what `!!` and the Up
//! walk need. It cannot answer the three questions a finder ranks on:
//!
//! * **how often** — `RunRow::runs`, the count that makes `git status` outrank a command typed
//!   once by accident three months ago;
//! * **when last** — `RunRow::last_at`, which is what the finder shows instead of the completion
//!   dropdown's kind badge. A command is identified by its text; what you want to know beside it
//!   is whether you ran it this morning or last spring;
//! * **where** — the directory it ran in, so a command from the project you are standing in can
//!   be ranked above the same command from somewhere else.
//!
//! All three are already written on every command, by the tracker that has always been there. The
//! finder is a reader; nothing new is recorded for it.
//!
//! # One row per command line, not per place it ran
//!
//! The store keys runs by `(directory, mode, argv)`, so `cargo build` run in four checkouts is
//! four rows. A finder listing it four times would be a worse list, so the rows are folded on the
//! command text: the counts add up, the newest timestamp wins, and the directory kept is the one
//! from the most recent run — the one worth showing beside it.

use super::db::{Track, read_dir};
use super::kv::{Span, Tree, Walk};
use super::row::{RunRow, field};
use std::collections::HashMap;

/// One command line, folded across every directory it has run in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The command as it was typed.
    pub line: String,
    /// `sh` or `lua`. Carried so the finder can hand a recalled line back to the right language.
    pub mode: String,
    /// How many times it has run, everywhere.
    pub runs: i64,
    /// Unix seconds of the most recent run.
    pub last_at: i64,
    /// The directory of the most recent run, for display and for ranking against the cwd.
    pub dir: String,
    /// How many distinct directories it has run in. `1` is a project-specific command; a large
    /// number is something you run everywhere.
    pub places: usize,
    /// Whether the most recent run succeeded. A command that has only ever failed is still worth
    /// listing — you may be going back to fix it — but it should not be ranked as a habit.
    pub worked: bool,
    /// The shell session of the most recent run, for the `session` scope.
    pub session: String,
    /// The machine of the most recent run, for the `host` scope.
    pub host: String,
    /// The git worktree the most recent run happened in, or `None` outside a repository.
    ///
    /// Taken from the directory's own row rather than by asking git at filter time: the store
    /// already resolved it when the directory was first visited, and re-walking five thousand
    /// directories per keystroke is not a filter anybody would wait for.
    pub root: Option<String>,
}

impl Track {
    /// Forget every run of `line` in `mode`, in every directory. Answers how many rows went.
    ///
    /// The finder's Delete key. A command you have run once by mistake — a password on the command
    /// line, a typo that will be suggested forever — should be removable without emptying the
    /// whole history, which is the only tool `history -c` gives you.
    ///
    /// Every directory, not just this one: the line is what you want gone, and leaving copies
    /// under other directories would make it reappear the moment you walked there.
    ///
    /// **And out of the log, not only the aggregate.** This used to touch [`Tree::Run`] alone, so
    /// the line stayed in [`Tree::History`] and came back through `recent()` on the next start —
    /// the password in the doc comment above included. The two stores answer different questions
    /// but "forget this" is a statement about the line, not about one index of it.
    pub fn forget(&self, line: &str, mode: &str) -> usize {
        let runs = self.forget_runs_of(line, mode);
        runs + self.forget_log_of(line, mode)
    }

    /// The aggregate's copies: `(dir_id, mode, argv)` for every directory it ran in.
    fn forget_runs_of(&self, line: &str, mode: &str) -> usize {
        let doomed: Vec<Vec<u8>> = self
            .store
            .read(|reader| {
                Some(reader.collect(Tree::Run, &Span::all(), |key, _| {
                    let same = field::argv_of_run(key).is_some_and(|argv| argv == line)
                        && field::mode_of_run(key)
                            .is_some_and(|found| found.as_ref() == mode.as_bytes());
                    same.then(|| key.to_vec())
                }))
            })
            .unwrap_or_default();
        if doomed.is_empty() {
            return 0;
        }
        self.store.delete_keys_in_chunks(Tree::Run, &doomed)
    }

    /// The log's copies: every time the line was typed, whenever that was.
    ///
    /// A full scan of the bucket, which is the same shape `trim` already uses and is paid on a
    /// keystroke nobody presses in a loop. Matched on the decoded row rather than on the key,
    /// because the log keys on an id and keeps the line in the value.
    fn forget_log_of(&self, line: &str, mode: &str) -> usize {
        let doomed: Vec<Vec<u8>> = self
            .store
            .read(|reader| {
                Some(reader.collect(Tree::History, &Span::all(), |key, value| {
                    let entry = super::log::entry_of(value)?;
                    (entry.line == line && entry.mode == mode).then(|| key.to_vec())
                }))
            })
            .unwrap_or_default();
        if doomed.is_empty() {
            return 0;
        }
        self.store.delete_keys_in_chunks(Tree::History, &doomed)
    }

    /// Every distinct command line the store knows, newest-first, capped at `limit`.
    ///
    /// One scan of the run table. That is deliberate: the finder reads this once when it opens and
    /// then filters in memory, so the cost is paid on the keystroke that opens it and never again
    /// while you type. Going back to the store per keystroke would put a transaction — and on a
    /// cold page cache, a disk read — between a character and its frame.
    ///
    /// `limit` bounds what is *returned*, not what is scanned, because the rows worth keeping are
    /// the most recent ones and that is not known until the scan is done. A store with a hundred
    /// thousand runs still folds to far fewer distinct lines.
    pub fn commands(&self, limit: usize) -> Vec<Command> {
        let mut folded: HashMap<(String, String), Command> = HashMap::new();
        // Directory ids resolve to paths, and the same id recurs across most rows — a monorepo's
        // runs are nearly all one directory — so the lookup is cached rather than repeated.
        let mut paths: HashMap<u64, (String, Option<String>)> = HashMap::new();

        self.store.read(|reader| {
            reader.scan(Tree::Run, &Span::all(), |key, value| {
                let Some(row) = RunRow::decode(value) else {
                    return Walk::On;
                };
                let (Some(argv), Some(dir_id)) = (field::argv_of_run(key), field::leading_id(key))
                else {
                    return Walk::On;
                };
                let Some(mode) = field::mode_of_run(key) else {
                    return Walk::On;
                };
                // The mode is stored as bytes because a key field is bytes; it is always `sh` or
                // `lua`, so anything that is not valid text is a row from a future schema and is
                // skipped rather than guessed at.
                let Ok(mode) = std::str::from_utf8(&mode) else {
                    return Walk::On;
                };
                let mode = mode.to_string();
                let argv = argv.to_string();
                if argv.is_empty() {
                    return Walk::On;
                }

                let (path, root) = paths.entry(dir_id).or_insert_with(|| {
                    read_dir(reader, dir_id)
                        .map(|dir| (dir.path, dir.root))
                        .unwrap_or_default()
                });

                folded
                    .entry((argv.clone(), mode.clone()))
                    .and_modify(|command| {
                        command.runs += row.runs;
                        command.places += 1;
                        // The newest run owns the directory and the status: it is the one the
                        // person is most likely to mean.
                        if row.last_at > command.last_at {
                            command.last_at = row.last_at;
                            command.dir = path.clone();
                            command.root = root.clone();
                            command.session = row.session.clone();
                            command.host = row.host.clone();
                            command.worked = row.worked();
                        }
                    })
                    .or_insert_with(|| Command {
                        line: argv,
                        mode: mode.clone(),
                        runs: row.runs,
                        last_at: row.last_at,
                        dir: path.clone(),
                        root: root.clone(),
                        session: row.session.clone(),
                        host: row.host.clone(),
                        places: 1,
                        worked: row.worked(),
                    });
                Walk::On
            });
            Some(())
        });

        let mut all: Vec<Command> = folded.into_values().collect();
        // Newest first, so a finder opened with an empty query shows what you just did. Ties break
        // on the run count and then on the text, so the order is total and the list does not
        // reshuffle between two openings of the same store.
        all.sort_by(|a, b| {
            b.last_at
                .cmp(&a.last_at)
                .then(b.runs.cmp(&a.runs))
                .then(a.line.cmp(&b.line))
        });
        all.truncate(limit);
        all
    }
}
