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
//! `turso` is the pure-Rust rewrite of SQLite, which is what keeps oslo's build free of a C
//! toolchain — `cargo tree -e build` returns only `oslo`, and the static musl binary still links.
//! Its API is async, and oslo's REPL is not. Rather than colour the shell async, every call below
//! blocks on a small current-thread runtime owned by this module. The runtime is built once: one
//! per call would be a thread and an epoll set per command.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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

/// An open history database.
pub struct History {
    db: turso::Database,
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
            Some(History { db })
        })
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
}
