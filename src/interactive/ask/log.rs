//! `ui log` — a line of output that says how serious it is.
//!
//! The smallest widget here and the one most likely to end up in every script: a level, a
//! timestamp if you want one, and the message. It goes to **stderr**, because a log line is not
//! the script's output — `ui log info "starting"` inside `$(…)` must not become part of the value.
//!
//! # Why a shell needs this at all
//!
//! Because the alternative is `echo "[ERROR] ..." >&2` in every script, and then the level is a
//! string somebody typed, the colour is an escape somebody pasted, and nothing can be filtered on
//! afterwards. A level that the shell knows about is one `--level` can suppress.

use crate::interactive::theme::{self, Color, Style};

/// How serious a line is. The set is small because a level nobody can order is not a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
    /// Like `error`, and then the shell stops. gum calls it `fatal`.
    Fatal,
}

impl Level {
    pub fn parse(name: &str) -> Option<Level> {
        Some(match name.to_ascii_lowercase().as_str() {
            "debug" => Level::Debug,
            "info" => Level::Info,
            "warn" | "warning" => Level::Warn,
            "error" | "err" => Level::Error,
            "fatal" => Level::Fatal,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
            Level::Fatal => "FATAL",
        }
    }

    /// The colour a level is drawn in, from the ordinary ANSI eight so it fits any palette.
    fn style(self, ui: &theme::Ui) -> Style {
        match self {
            Level::Debug => ui.muted,
            Level::Info => Style::fg(Color::Basic {
                index: 6,
                bright: true,
            }),
            Level::Warn => Style::fg(Color::Basic {
                index: 3,
                bright: true,
            }),
            Level::Error | Level::Fatal => ui.error,
        }
    }
}

/// How a line is rendered.
#[derive(Debug, Clone)]
pub struct Entry {
    pub level: Level,
    pub message: String,
    /// Shown before the level. The caller supplies it rather than this reading the clock, so the
    /// renderer stays a pure function and a test can assert on the whole line.
    pub time: Option<String>,
    /// `key=value` pairs after the message, as structured logging has them.
    pub fields: Vec<(String, String)>,
}

/// The line to write, without a trailing newline.
pub fn line(entry: &Entry) -> String {
    let theme = theme::current();
    let depth = theme::depth();
    let mut out = String::new();

    if let Some(time) = &entry.time {
        out.push_str(&theme.ui.muted.paint(time, depth));
        out.push(' ');
    }
    // Padded, so the messages of a run of lines start in the same column and can be read down.
    out.push_str(
        &entry
            .level
            .style(&theme.ui)
            .paint(&format!("{:<5}", entry.level.label()), depth),
    );
    out.push(' ');
    out.push_str(&entry.message);

    for (key, value) in &entry.fields {
        out.push(' ');
        out.push_str(&theme.ui.accent.paint(key, depth));
        out.push_str(&theme.ui.muted.paint("=", depth));
        // Quoted only when it needs to be, which keeps the common case readable and the awkward
        // case unambiguous.
        if value.contains(char::is_whitespace) || value.is_empty() {
            out.push_str(&format!("{value:?}"));
        } else {
            out.push_str(value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(rendered: &str) -> String {
        let mut out = String::new();
        let mut chars = rendered.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        }
        out
    }

    fn entry(level: Level, message: &str) -> Entry {
        Entry {
            level,
            message: message.to_string(),
            time: None,
            fields: Vec::new(),
        }
    }

    /// The level is padded so a run of lines has its messages in one column.
    #[test]
    fn levels_line_up() {
        let info = plain(&line(&entry(Level::Info, "x")));
        let error = plain(&line(&entry(Level::Error, "x")));
        assert_eq!(info, "INFO  x");
        assert_eq!(error, "ERROR x");
        assert_eq!(info.find('x'), error.find('x'), "messages must align");
    }

    #[test]
    fn a_timestamp_comes_first() {
        let mut e = entry(Level::Warn, "late");
        e.time = Some("12:00:00".to_string());
        assert_eq!(plain(&line(&e)), "12:00:00 WARN  late");
    }

    /// Fields are `key=value`, quoted only when the value would otherwise be ambiguous.
    #[test]
    fn fields_are_quoted_only_when_they_need_to_be() {
        let mut e = entry(Level::Info, "done");
        e.fields = vec![
            ("id".to_string(), "42".to_string()),
            ("name".to_string(), "two words".to_string()),
            ("empty".to_string(), String::new()),
        ];
        assert_eq!(
            plain(&line(&e)),
            r#"INFO  done id=42 name="two words" empty="""#
        );
    }

    /// Levels order, which is what `--level` filters on.
    #[test]
    fn levels_are_ordered() {
        assert!(Level::Debug < Level::Info);
        assert!(Level::Info < Level::Warn);
        assert!(Level::Warn < Level::Error);
        assert!(Level::Error < Level::Fatal);
    }

    #[test]
    fn level_names_are_read_case_insensitively() {
        assert_eq!(Level::parse("INFO"), Some(Level::Info));
        assert_eq!(Level::parse("warning"), Some(Level::Warn));
        assert_eq!(Level::parse("err"), Some(Level::Error));
        assert_eq!(Level::parse("loud"), None);
    }
}
