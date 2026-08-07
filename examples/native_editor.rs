//! Drive oslo's native line editor on a real terminal, without the shell around it.
//!
//! ```sh
//! cargo run --example native_editor
//! ```
//!
//! Phase 1 of replacing rustyline: this is the editor with a stub `Assist`, so what you are trying
//! is the buffer, the layout and the redraw — the parts that own the row. Highlighting, the
//! completion dropdown and real history come later, through the same trait.
//!
//! Type, edit, press Enter to see the line echoed, Ctrl-C to abandon one, Ctrl-D on an empty line
//! to leave.

use oslo::ui::edit::session::{Assist, Outcome, read_line};

/// Just enough to show the wiring works: a fixed history and a ghost hint drawn from it.
#[derive(Default)]
struct Demo {
    history: Vec<String>,
    at: usize,
    typed: Option<String>,
}

impl Assist for Demo {
    fn highlight(&mut self, line: &str) -> String {
        // The first word in green, so it is obvious the layout measures the plain text and not
        // the escapes: the cursor must stay put as the colour comes and goes.
        match line.split_once(' ') {
            Some((head, rest)) => format!("\x1b[32m{head}\x1b[0m {rest}"),
            None => format!("\x1b[32m{line}\x1b[0m"),
        }
    }

    fn hint_text(&mut self, line: &str, _cursor: usize) -> Option<String> {
        if line.is_empty() {
            return None;
        }
        let found = self.history.iter().find(|h| h.starts_with(line))?;
        Some(found[line.len()..].to_string())
    }

    fn paint_hint(&mut self, text: &str) -> String {
        format!("\x1b[90m{text}\x1b[0m")
    }

    fn history_prev(&mut self, line: &str) -> Option<String> {
        let entry = self.history.iter().rev().nth(self.at)?.clone();
        if self.at == 0 {
            self.typed = Some(line.to_string());
        }
        self.at += 1;
        Some(entry)
    }

    fn history_next(&mut self) -> Option<String> {
        match self.at {
            0 => None,
            1 => {
                self.at = 0;
                self.typed.take()
            }
            _ => {
                self.at -= 1;
                self.history.iter().rev().nth(self.at - 1).cloned()
            }
        }
    }
}

fn main() {
    let mut assist = Demo {
        history: vec![
            "cargo test --all".to_string(),
            "git status".to_string(),
            "ls -la /tmp".to_string(),
        ],
        ..Demo::default()
    };
    println!("oslo native editor — Enter echoes, Ctrl-C abandons, Ctrl-D on an empty line exits.");
    loop {
        assist.at = 0;
        // A function rather than a string: the editor rebuilds the prompt when something it shows
        // has changed, which is what lets a vi-mode indicator be right. This one never changes.
        let mut render = || {
            (
                "\x1b[1;35mnative\x1b[0m ❯ ".to_string(),
                "\x1b[90mdemo\x1b[0m".to_string(),
            )
        };
        match read_line(&mut render, ("", 0), &mut assist) {
            Outcome::Line(line) => {
                if line.trim() == "exit" {
                    return;
                }
                println!("  got: {line:?}");
                if !line.trim().is_empty() {
                    assist.history.push(line);
                }
            }
            Outcome::Interrupted => println!("  (interrupted)"),
            // The demo has one language, so there is nothing to switch to.
            Outcome::ToggleLanguage { .. } => println!("  (toggle: nothing to switch to here)"),
            Outcome::Eof => return,
        }
    }
}
