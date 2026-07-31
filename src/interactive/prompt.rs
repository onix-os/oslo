//! The prompt: what the shell draws before the line, and to the right of it.
//!
//! A prompt is a Lua function (see `crate::lua::api::prompt`); everything here is either what a
//! prompt function calls, or the built-in prompt used when no Lua one is set.
//!
//! **The right prompt is the interesting part.** rustyline has no support for one, and it repaints
//! from the prompt to end-of-line on every keystroke — which is why the previous attempt was
//! deleted rather than fixed. What makes it work is where the repaint clears: `refresh_line`
//! clears the old rows *before* writing the prompt and never clears afterwards, so anything the
//! prompt string draws survives the redraw and is rewritten each time. And rustyline measures a
//! prompt by counting graphemes with every CSI sequence as zero width, so an absolute column move
//! wrapped in save/restore costs nothing in its arithmetic.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use super::theme;

/// The branch the working directory is on, or a short hash when detached.
pub fn git_branch() -> Option<String> {
    let head = git_root()?.join(".git/HEAD");
    let content = fs::read_to_string(head).ok()?;
    let trimmed = content.trim();
    match trimmed.strip_prefix("ref: refs/heads/") {
        Some(branch) => Some(branch.to_string()),
        // Detached: `HEAD` holds the commit itself, and seven characters is what everyone shows.
        None if trimmed.len() >= 7 => Some(trimmed[..7].to_string()),
        None => None,
    }
}

/// The top of the working tree the current directory is in.
pub fn git_root() -> Option<PathBuf> {
    let current = env::current_dir().ok()?;
    let mut dir: &Path = current.as_path();
    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// A path with `$HOME` written as `~`.
pub fn tilde(path: &str) -> String {
    let home = env::var("HOME").unwrap_or_default();
    if home.is_empty() || !path.starts_with(&home) {
        return path.to_string();
    }
    // Only at a component boundary: `/home/bo` must not become `~` when `$HOME` is `/home/b`.
    let rest = &path[home.len()..];
    if rest.is_empty() || rest.starts_with('/') {
        format!("~{rest}")
    } else {
        path.to_string()
    }
}

/// `~/d/o/t/rush` — every component but the last `keep` cut to its first character.
pub fn shorten(path: &str, keep: usize) -> String {
    let path = tilde(path);
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= keep {
        return path;
    }
    let cut = parts.len() - keep;
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            if i >= cut || part.is_empty() {
                return (*part).to_string();
            }
            // A leading dot is kept with the letter after it, or `.config` becomes `.` and every
            // dotted directory looks the same.
            let mut chars = part.chars();
            match chars.next() {
                Some('.') => match chars.next() {
                    Some(second) => format!(".{second}"),
                    None => ".".to_string(),
                },
                Some(first) => first.to_string(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The built-in left prompt, used when no Lua one is set.
pub fn render_default_left_prompt(last_status: i32) -> String {
    let theme = theme::current();
    let depth = theme::depth();

    let pwd = tilde(
        &env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "/".to_string()),
    );

    let mut out = theme.prompt.cwd.paint(&pwd, depth);
    if let Some(branch) = git_branch() {
        out.push_str(&theme.prompt.git.paint(&format!(" ({branch})"), depth));
    }
    let arrow = if last_status == 0 {
        theme.prompt.ok
    } else {
        theme.prompt.failed
    };
    out.push(' ');
    out.push_str(&arrow.paint("❯", depth));
    out.push(' ');
    out
}

/// The escape that draws a right prompt, or empty when there is no room for one.
///
/// **Where this goes matters, and the obvious place is wrong.** Putting it in the prompt string
/// corrupts rustyline's cursor arithmetic: it counts a CSI sequence as zero width but has no idea
/// that `\x1b[76G` *moves* the cursor, so the right prompt's own characters get added to the
/// column it thinks the line starts at. Measured, the composed prompt came out eleven cells wide
/// where the left prompt alone is six.
///
/// The seam that works is the highlighter. `compute_layout` measures the **raw line**, never the
/// string `highlight` returns — so anything drawn there is free of the arithmetic entirely. And
/// `refresh_line` clears the old rows *before* it writes, and never after, so this is redrawn
/// intact on every keystroke rather than erased by the next one.
///
/// Empty when it will not fit. A right prompt that collides with what is being typed is worse
/// than none, and that collision is what made the previous attempt look broken.
pub fn right_prompt_escape(right: &str, used: usize, cols: usize) -> String {
    if right.is_empty() || cols == 0 {
        return String::new();
    }
    let right_w = printed_width(right);
    // Two columns of gap, so the text being typed never touches it.
    if used + right_w + 2 > cols {
        return String::new();
    }
    let column = cols - right_w + 1;
    // Save, jump to the column that ends it flush with the screen edge, draw, restore — so the
    // cursor is physically where it started and the hint drawn after this lands in the right
    // place.
    format!("\x1b7\x1b[{column}G{right}\x1b8")
}

/// How many cells a string occupies once its escape sequences are discounted.
///
/// `dropdown::display_width` cannot be used here: it documents itself as assuming plain text, and
/// a prompt is never plain — it is mostly colour escapes. Counting those as cells would push the
/// right prompt left by however many bytes of colour the left prompt happened to carry, which is
/// a bug that only appears once somebody themes their prompt.
///
/// The rule is rustyline's own, deliberately: an `\x1b` starts a sequence, `[` makes it a CSI that
/// runs to its first non-digit non-`;` byte, and anything else is a two-character sequence. Both
/// sides have to agree about the width or the cursor lands in the wrong column.
pub fn printed_width(text: &str) -> usize {
    let mut width = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            width += super::dropdown::display_width(&c.to_string());
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                // A CSI runs to its final byte, which is the first that is not a parameter.
                for c in chars.by_ref() {
                    if !c.is_ascii_digit() && c != ';' {
                        break;
                    }
                }
            }
            // `\x1b7`, `\x1b8` and friends: two characters in total.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_under_home_is_written_with_a_tilde() {
        // SAFETY: a test-local variable, read only by the functions under test.
        unsafe { env::set_var("HOME", "/home/someone") };
        assert_eq!(tilde("/home/someone/src"), "~/src");
        assert_eq!(tilde("/home/someone"), "~");
        // Only at a component boundary: a different user whose name starts the same is not home.
        assert_eq!(tilde("/home/someoneelse/src"), "/home/someoneelse/src");
        assert_eq!(tilde("/etc"), "/etc");
    }

    #[test]
    fn shortening_keeps_the_tail_and_abbreviates_the_rest() {
        unsafe { env::set_var("HOME", "/home/someone") };
        assert_eq!(shorten("/home/someone/data/code/rush", 1), "~/d/c/rush");
        assert_eq!(shorten("/home/someone/data/code/rush", 2), "~/d/code/rush");
        // A dotted directory keeps the letter after the dot, or every one of them looks alike.
        assert_eq!(shorten("/home/someone/.config/oslo", 1), "~/.c/oslo");
        // Nothing to cut.
        assert_eq!(shorten("/etc", 1), "/etc");
    }

    /// Drawn flush with the right edge, and returning the cursor to where it was so that whatever
    /// is written next — the ghost hint — still lands at the cursor.
    #[test]
    fn a_right_prompt_is_drawn_flush_right_and_restores_the_cursor() {
        let escape = right_prompt_escape("12:34", 10, 80);
        // 80 - 5 + 1: the last cell of `12:34` falls on column 80.
        assert!(escape.starts_with("\x1b7\x1b[76G"), "{escape:?}");
        assert!(escape.ends_with("\x1b8"), "{escape:?}");
        assert!(escape.contains("12:34"), "{escape:?}");
    }

    /// A prompt is mostly colour escapes, so measuring it as plain text would push the right
    /// prompt left by however many bytes of colour the left one carried.
    #[test]
    fn width_counts_cells_and_not_escapes() {
        assert_eq!(printed_width("abc"), 3);
        assert_eq!(printed_width("\x1b[1;32mabc\x1b[0m"), 3);
        assert_eq!(printed_width("\x1b7\x1b[70Gx\x1b8abc"), 4);
        // And a coloured right prompt is positioned by its cells, not its bytes.
        let plain = right_prompt_escape("12:34", 0, 80);
        let painted = right_prompt_escape("\x1b[90m12:34\x1b[0m", 0, 80);
        let column = |s: &str| s.split('G').next().map(str::to_string);
        assert_eq!(column(&plain), column(&painted));
    }

    /// A right prompt that would collide with what is being typed is worse than none — that
    /// collision is what made the previous attempt look broken.
    #[test]
    fn a_right_prompt_is_dropped_when_it_will_not_fit() {
        // The line has grown to within two columns of it.
        assert_eq!(right_prompt_escape("12:34", 74, 80), "");
        assert_eq!(right_prompt_escape("12:34", 0, 0), "");
        assert_eq!(right_prompt_escape("", 0, 80), "");
        // With room, it is drawn.
        assert!(!right_prompt_escape("12:34", 10, 80).is_empty());
    }
}
