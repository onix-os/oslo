//! Saying what the directory environment did, in a way worth reading.
//!
//! The first version printed one `direnv:` line per event and let the rc file's own output fall
//! wherever it landed, which on a real `.envrc` meant eight `oslo: use: command not found` lines
//! above a summary that did not mention them. Everything here is about that: the rc file's output
//! belongs *under* the file that produced it, repeated lines belong collapsed, and the one line
//! that is a security decision belongs looking like one.

use oslo::direnv::Event;
use oslo::interactive::theme::{self, Color, Style};
use std::path::Path;

/// The label every line starts with, so the block reads as one thing.
const LABEL: &str = "direnv";

fn paint(text: &str, style: Style) -> String {
    style.paint(text, theme::depth())
}

/// The ANSI slots rather than absolute colours, deliberately.
///
/// The syntax palette is pinned to RGB so a wallpaper tool cannot repaint the shell's idea of "this
/// command does not exist". This is not that: these are ordinary status messages, and they should
/// follow the terminal's own scheme the way the prompt and the pager already do.
fn slot(index: u8) -> Color {
    Color::Basic {
        index,
        bright: false,
    }
}

const RED: u8 = 1;
const GREEN: u8 = 2;
const YELLOW: u8 = 3;

/// `$HOME/data/code` as `~/data/code`, because the full path is rarely the interesting part.
fn short(path: &Path) -> String {
    let text = path.to_string_lossy().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && text.starts_with(&home) => {
            format!("~{}", &text[home.len()..])
        }
        _ => text,
    }
}

/// Repeated lines collapsed to one with a count.
///
/// A real `.envrc` calling `export_alias` four times produces four identical errors, and four
/// copies of one fact is three lines of noise. Order is preserved — the first occurrence keeps its
/// place — because the sequence is how you work out which line of the file failed.
fn collapse(output: &str) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for line in output.lines().map(str::trim_end).filter(|l| !l.is_empty()) {
        // The shell's own errors already say `oslo:`; inside this block that prefix is noise, since
        // the block is already labelled with the file that caused them.
        let line = line.strip_prefix("oslo: ").unwrap_or(line);
        match out.iter_mut().find(|(seen, _)| seen == line) {
            Some((_, count)) => *count += 1,
            None => out.push((line.to_string(), 1)),
        }
    }
    out
}

/// Print the rc file's own output, indented under the file it came from.
pub(super) fn detail(output: &str) {
    let rail = paint("│", Style::fg(Color::Indexed(240)));
    let dim = Style {
        dim: true,
        ..Style::default()
    };
    for (line, count) in collapse(output) {
        let times = match count {
            1 => String::new(),
            n => paint(&format!("  ×{n}"), Style::fg(Color::Indexed(240))),
        };
        println!("  {rail} {}{times}", paint(&line, dim));
    }
}

/// One event, as one line.
pub(super) fn event(event: &Event) {
    let grey = Style::fg(Color::Indexed(240));
    match event {
        Event::Loaded { owner, vars } => {
            let label = paint(LABEL, Style::fg(slot(GREEN)));
            let count = match vars {
                1 => "1 variable".to_string(),
                n => format!("{n} variables"),
            };
            println!(
                "{label} {} {}",
                paint(&short(owner), Style::default()),
                paint(&format!("· {count}"), grey)
            );
        }
        Event::Unloaded { owner } => {
            println!(
                "{} {}",
                paint(LABEL, grey),
                paint(&format!("left {}", short(owner)), grey)
            );
        }
        // The one line here that is a security decision, so it is the one that gets a colour you
        // cannot skim past, and it says what to type rather than making you remember.
        Event::Blocked { path } => {
            let label = paint(
                LABEL,
                Style {
                    bold: true,
                    ..Style::fg(slot(YELLOW))
                },
            );
            println!(
                "{label} {} {}",
                paint(&short(path), Style::default()),
                paint("is not allowed yet", grey)
            );
            println!(
                "  {} {}",
                paint("→", grey),
                paint(
                    "direnv allow",
                    Style {
                        bold: true,
                        ..Style::fg(slot(YELLOW))
                    }
                )
            );
        }
        Event::Denied { path } => {
            println!(
                "{} {} {}",
                paint(LABEL, Style::fg(slot(RED))),
                paint(&short(path), Style::default()),
                paint("is denied", grey)
            );
        }
        Event::Failed { path, problem } => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| short(path));
            println!(
                "{} {} {}",
                paint(LABEL, Style::fg(slot(RED))),
                paint(&name, Style::default()),
                paint(problem, grey)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four identical errors are one fact, not four.
    #[test]
    fn repeated_lines_collapse_with_a_count() {
        let output = "oslo: export_alias: command not found\n\
                      oslo: export_alias: command not found\n\
                      oslo: use: command not found\n\
                      oslo: export_alias: command not found\n";
        assert_eq!(
            collapse(output),
            vec![
                ("export_alias: command not found".to_string(), 3),
                ("use: command not found".to_string(), 1),
            ],
            "and the first occurrence keeps its place, so the order still reads as the file did"
        );
    }

    /// The `oslo:` prefix is dropped, because the block already says whose output this is.
    #[test]
    fn the_shell_prefix_comes_off_inside_the_block() {
        assert_eq!(
            collapse("oslo: use: command not found\n"),
            vec![("use: command not found".to_string(), 1)]
        );
    }

    #[test]
    fn blank_lines_are_not_output() {
        assert_eq!(collapse("\n\n  \n"), vec![]);
    }
}
