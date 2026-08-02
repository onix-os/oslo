//! Command history in a database rather than a flat file.
//!
//! A text file cannot answer the question the shell actually has. oslo reads two languages, and a
//! line recalled from history has to run in the one it was typed in — recall a Lua line while the
//! prompt is in shell mode and a flat file gives you no way to know. The mode is a column here, so
//! there is nothing to guess and no marker smuggled into the text.
//!
//! # Where it lives
//!
//! `$XDG_DATA_HOME/oslo/history.db`, falling back to `~/.local/share/oslo/history.db`. History is
//! state the user accumulates, not configuration they wrote, so it belongs under the data
//! directory rather than in `$HOME` or beside the config.
//!
//! # Why the async is hidden here
//!
//! `turso` is the pure-Rust rewrite of SQLite, so there is no vendored SQLite here. It is not,
//! however, free of a C toolchain: `cargo tree -i cc` names `aegis`, `simsimd`, `libmimalloc-sys`
//! and `zstd-sys`, all reached through `turso_core`, and every one of them shells out to a
//! compiler. `cargo tree -e build` says otherwise only because it lists oslo's *direct* build
//! dependencies, of which there are none. The release workflow installs `musl-tools` for them.
//! Its API is async, and oslo's REPL is not. Rather than colour the shell async, every call below
//! blocks on a small current-thread runtime owned by this module. The runtime is built once: one
//! per call would be a thread and an epoll set per command.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The language a line was typed in, as stored.
///
/// A string rather than an integer so the table can be read by hand — `sqlite3 history.db 'select
/// * from history'` should be legible without a decoder ring.
pub const MODE_SHELL: &str = "sh";
pub const MODE_LUA: &str = "lua";

/// One line, as it was typed and in the language it was typed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub line: String,
    pub mode: String,
}

/// Where history is kept, given the environment.
///
/// `$XDG_DATA_HOME` first, then the specification's own default of `~/.local/share`. Returns
/// `None` when neither is knowable, which is a shell with no home — a container's `nobody`, say —
/// and which must run without a history rather than fail.
pub fn database_path(xdg_data: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg_data {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(home?).join(".local/share"),
    };
    Some(base.join("oslo/history.db"))
}

/// The runtime every call blocks on. See the module note.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime")
    })
}

/// The schema. `IF NOT EXISTS` because every session runs it; there is no migration step.
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS history (\
     id INTEGER PRIMARY KEY AUTOINCREMENT, \
     line TEXT NOT NULL, \
     mode TEXT NOT NULL, \
     at INTEGER NOT NULL)";

/// How many appends go by between trims. See [`History::trim_soon`].
const TRIM_EVERY: usize = 100;

/// An open history database.
pub struct History {
    db: turso::Database,
    /// Appends since the last trim, so the scan is amortised rather than paid per line.
    since_trim: AtomicUsize,
}

impl History {
    /// Open, creating the file and its directory if they are not there.
    ///
    /// Every failure answers `None` rather than propagating: a shell whose history cannot be
    /// opened is a working shell without history, and refusing to start over it would be absurd.
    pub fn open(path: &Path) -> Option<History> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }
        let path = path.to_str()?.to_string();
        runtime().block_on(async move {
            let db = turso::Builder::new_local(&path).build().await.ok()?;
            let conn = db.connect().ok()?;
            conn.execute(SCHEMA, ()).await.ok()?;
            Some(History {
                db,
                since_trim: AtomicUsize::new(0),
            })
        })
    }

    /// Trim, but not more often than one line in [`TRIM_EVERY`].
    ///
    /// The REPL used to call [`History::trim`] after every single command, and `trim` is a
    /// `DELETE ... WHERE id NOT IN (SELECT ... LIMIT N)` — a full scan of the table, per line
    /// typed, to delete nothing at all in the overwhelming majority of cases. A hundred lines of
    /// slack against a ten-thousand-line bound is not a bound anybody can perceive, and the scan
    /// the shell gets back pays for everything else it now does with a database per command.
    ///
    /// The loop trims unconditionally on the way out, so a short session still ends bounded.
    pub fn trim_soon(&self, max: usize) {
        if self.since_trim.fetch_add(1, Ordering::Relaxed) + 1 >= TRIM_EVERY {
            self.since_trim.store(0, Ordering::Relaxed);
            self.trim(max);
        }
    }

    /// Fold the write-ahead log back into the database and truncate it. Best effort.
    ///
    /// turso opens in WAL mode without being asked, which is why a second terminal can read this
    /// table while this one appends to it — and it never checkpoints on its own, not on drop and not
    /// on reopen. So the `-wal` grows by a page or two per line typed and is never given back:
    /// measured 330 KB of log against 4 KB of data after one short session. It has to be asked for,
    /// and the way out of the loop is the place to ask.
    ///
    /// Through `query` rather than `execute`: a checkpoint answers with a row, and `execute` fails
    /// on a statement that returns one.
    pub fn checkpoint(&self) {
        runtime().block_on(async {
            let Ok(conn) = self.db.connect() else {
                return;
            };
            if let Ok(mut rows) = conn.query("PRAGMA wal_checkpoint(TRUNCATE)", ()).await {
                // Stepped, not merely offered: turso runs the statement as the row is fetched, so a
                // `Rows` that is dropped unread checkpoints nothing.
                let _ = rows.next().await;
            }
        });
    }

    /// Remember one line.
    pub fn append(&self, line: &str, mode: &str) -> bool {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        runtime().block_on(async {
            let Ok(conn) = self.db.connect() else {
                return false;
            };
            conn.execute(
                "INSERT INTO history (line, mode, at) VALUES (?1, ?2, ?3)",
                (line, mode, at),
            )
            .await
            .is_ok()
        })
    }

    /// The most recent `limit` lines, oldest first — the order a line editor wants them in.
    pub fn recent(&self, limit: usize) -> Vec<Entry> {
        runtime().block_on(async {
            let Ok(conn) = self.db.connect() else {
                return Vec::new();
            };
            let Ok(mut rows) = conn
                .query(
                    "SELECT line, mode FROM history ORDER BY id DESC LIMIT ?1",
                    (limit as i64,),
                )
                .await
            else {
                return Vec::new();
            };
            let mut out = Vec::new();
            while let Ok(Some(row)) = rows.next().await {
                match (row.get_value(0), row.get_value(1)) {
                    (Ok(turso::Value::Text(line)), Ok(turso::Value::Text(mode))) => {
                        out.push(Entry { line, mode });
                    }
                    _ => break,
                }
            }
            out.reverse();
            out
        })
    }

    /// Drop everything. `history -c`.
    pub fn clear(&self) -> bool {
        runtime().block_on(async {
            let Ok(conn) = self.db.connect() else {
                return false;
            };
            conn.execute("DELETE FROM history", ()).await.is_ok()
        })
    }

    /// Trim to the newest `max` lines, which is what `$HISTSIZE` asks for.
    pub fn trim(&self, max: usize) -> bool {
        runtime().block_on(async {
            let Ok(conn) = self.db.connect() else {
                return false;
            };
            conn.execute(
                "DELETE FROM history WHERE id NOT IN \
                 (SELECT id FROM history ORDER BY id DESC LIMIT ?1)",
                (max as i64,),
            )
            .await
            .is_ok()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// History is state the user accumulates, not configuration they wrote, so it goes under the
    /// data directory — not `$HOME`, and not beside the config.
    #[test]
    fn the_database_lives_under_the_data_directory() {
        assert_eq!(
            database_path(Some("/x/data"), Some("/home/u")),
            Some(PathBuf::from("/x/data/oslo/history.db"))
        );
        // No XDG: the specification's own default.
        assert_eq!(
            database_path(None, Some("/home/u")),
            Some(PathBuf::from("/home/u/.local/share/oslo/history.db"))
        );
        // An empty XDG is unset, not a relative path from the root.
        assert_eq!(
            database_path(Some("  "), Some("/home/u")),
            Some(PathBuf::from("/home/u/.local/share/oslo/history.db"))
        );
        // Nowhere to put it is not an error; it is a shell without history.
        assert_eq!(database_path(None, None), None);
    }

    fn temp_db() -> (tempfile::TempDir, History) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("nested/history.db");
        let history = History::open(&path).expect("the database opens");
        (dir, history)
    }

    /// The whole reason for a database: the language survives the round trip, so recalling a Lua
    /// line while the prompt is in shell mode does not run it as shell.
    #[test]
    fn a_line_remembers_which_language_it_was_typed_in() {
        let (_dir, history) = temp_db();
        assert!(history.append("ls -la", MODE_SHELL));
        assert!(history.append("print(1)", MODE_LUA));

        let entries = history.recent(10);
        assert_eq!(
            entries,
            vec![
                Entry {
                    line: "ls -la".to_string(),
                    mode: MODE_SHELL.to_string()
                },
                Entry {
                    line: "print(1)".to_string(),
                    mode: MODE_LUA.to_string()
                },
            ],
            "oldest first, each with its own mode"
        );
    }

    /// Opening creates the directory as well as the file — a fresh machine has neither.
    #[test]
    fn opening_creates_what_is_missing() {
        let (dir, history) = temp_db();
        assert!(dir.path().join("nested/history.db").exists());
        assert!(history.recent(10).is_empty(), "a new database is empty");
    }

    #[test]
    fn recent_returns_the_newest_and_trimming_keeps_them() {
        let (_dir, history) = temp_db();
        for i in 1..=20 {
            history.append(&format!("cmd {i}"), MODE_SHELL);
        }
        let last_three = history.recent(3);
        assert_eq!(last_three.len(), 3);
        assert_eq!(last_three[2].line, "cmd 20", "newest is last");
        assert_eq!(last_three[0].line, "cmd 18");

        assert!(history.trim(5));
        let kept = history.recent(100);
        assert_eq!(kept.len(), 5, "trimming leaves the newest");
        assert_eq!(kept[0].line, "cmd 16");

        assert!(history.clear());
        assert!(history.recent(10).is_empty());
    }

    /// The amortised trim is still a bound: it lets the table run over for a while and then puts
    /// it back, rather than scanning the whole table once per line to delete nothing.
    #[test]
    fn the_bound_is_enforced_in_batches_rather_than_per_line() {
        let (_dir, history) = temp_db();
        for i in 1..=(TRIM_EVERY - 1) {
            history.append(&format!("cmd {i}"), MODE_SHELL);
            history.trim_soon(5);
        }
        assert_eq!(
            history.recent(1000).len(),
            TRIM_EVERY - 1,
            "nothing has been scanned yet"
        );

        history.append("cmd last", MODE_SHELL);
        history.trim_soon(5);
        let kept = history.recent(1000);
        assert_eq!(kept.len(), 5);
        assert_eq!(kept[4].line, "cmd last", "and it kept the newest");
    }
}
