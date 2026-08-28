//! Saying what the directory environment did, in a way worth reading.
//!
//! The first version printed one `direnv:` line per event and let the rc file's own output fall
//! wherever it landed, which on a noisy file meant eight loose error lines
//! above a summary that did not mention them. Everything here is about that: the rc file's output
//! belongs *under* the file that produced it, repeated lines belong collapsed, and the one line
//! that is a security decision belongs looking like one.

use oslo_shell::direnv::Event;
use oslo_shell::direnv::diff::Change;
use oslo_ui::block::Block;
use oslo_ui::theme::{self, Color, Style};
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
/// A file calling the same failing thing four times produces four identical errors, and four
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
    let mut block = Block::new("");
    for (line, count) in collapse(output) {
        let times = match count {
            1 => String::new(),
            n => format!("  ×{n}"),
        };
        block.note(format!("{line}{times}"));
    }
    print(&block);
}

/// The block, one write.
///
/// One `print!` rather than a `println!` per row: a block assembled across several statements must
/// not interleave with whatever the rc file itself is writing, and a partial block on the screen is
/// worse than a late one.
fn print(block: &Block) {
    let lines = block.lines();
    if lines.is_empty() {
        return;
    }
    println!("{}", lines.join("\n"));
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

/// The environment a directory changed, grouped by what happened to it.
///
/// One line per kind, on the same rail the failure detail uses, so a load and a failure read as
/// parts of one block rather than two unrelated shapes. Kinds are separate lines rather than one
/// mixed run because the eye groups by colour badly and by position well — thirty names in three
/// colours is a wall; three labelled rows is a list.
///
/// Each row is cut to the terminal with a count of what was dropped. A Nix dev shell adds
/// thirty-five variables, and printed in full that is four wrapped lines of noise on every `cd` —
/// which is how a useful message becomes one nobody reads.
fn rail_rows(
    block: &mut Block,
    changed: &[(String, Change)],
    aliases: &[(String, Change)],
    functions: &[String],
) {
    let grey = Style::fg(Color::Indexed(240));

    for kind in [Change::Removed, Change::Modified, Change::Added] {
        let names: Vec<&str> = changed
            .iter()
            .filter(|(_, k)| *k == kind)
            .map(|(name, _)| name.as_str())
            .collect();
        if names.is_empty() {
            continue;
        }
        let (style, label) = match kind {
            Change::Removed => (Style::fg(slot(RED)), "removed"),
            Change::Modified => (Style::fg(slot(YELLOW)), "changed"),
            Change::Added => (Style::fg(slot(GREEN)), "added"),
        };
        // `Count` is the default and the right one here: the names are a list, and past the edge
        // of the terminal the number of them is the information rather than the next name.
        block.styled_row(label, style, names.join(" "), Style::default());
    }

    if !aliases.is_empty() {
        let names: Vec<&str> = aliases.iter().map(|(name, _)| name.as_str()).collect();
        block.styled_row("aliases", grey, names.join(" "), Style::default());
    }

    // **The row a sourced shell file needs.** A `.env.lua` that reads somebody's `~/.profile` or a
    // pile of helpers defines most of what it defines as functions, and without this the block
    // announced the two variables and said nothing about the twelve functions — which are removed
    // again on the way out just the same, and so are worth the same line.
    if !functions.is_empty() {
        block.styled_row("functions", grey, functions.join(" "), Style::default());
    }
}

/// One event, as one block.
///
/// **The config gets first refusal.** `on-report` is handed the same fields this renders from, and
/// a handler that says it drew the event stops this drawing anything — see
/// `startup::report::handled`. Everything below is the default, for a shell whose config has no
/// opinion.
pub(super) fn event(event: &Event) {
    if crate::startup::report::handled(event) {
        return;
    }
    let grey = Style::fg(Color::Indexed(240));
    match event {
        Event::Loaded {
            owner,
            changed,
            aliases,
            functions,
        } => {
            let label = paint(LABEL, Style::fg(slot(GREEN)));
            let mut block = Block::new(format!(
                "{label} {}",
                paint(&short(owner), Style::default())
            ));
            rail_rows(&mut block, changed, aliases, functions);
            print(&block);
        }
        Event::Unloaded { owner } => {
            print(&Block::new(format!(
                "{} {}",
                paint(LABEL, grey),
                paint(&format!("left {}", short(owner)), grey)
            )));
        }
        // The one line here that is a security decision, so it is the one that gets a colour you
        // cannot skim past, and it says what to type rather than making you remember.
        Event::Blocked { path } => {
            let loud = Style {
                bold: true,
                ..Style::fg(slot(YELLOW))
            };
            let mut block = Block::new(format!(
                "{} {} {}",
                paint(LABEL, loud),
                paint(&short(path), Style::default()),
                paint("is not allowed yet", grey)
            ));
            block.styled_row("→", grey, "direnv allow", loud);
            print(&block);
        }
        Event::Denied { path } => {
            print(&Block::new(format!(
                "{} {} {}",
                paint(LABEL, Style::fg(slot(RED))),
                paint(&short(path), Style::default()),
                paint("is denied", grey)
            )));
        }
        Event::Failed { path, problem } => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| short(path));
            print(&Block::new(format!(
                "{} {} {}",
                paint(LABEL, Style::fg(slot(RED))),
                paint(&name, Style::default()),
                paint(problem, grey)
            )));
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
