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
use std::time::Duration;

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
/// `user@host | N | sh ❯`
///
/// Three segments, each answering a question the others cannot: **who and where** you are logged
/// in, **which editing mode** the keyboard is in, and **which language** the line will be read as.
/// The last matters more in oslo than in any other shell — the same characters mean different
/// things in shell and in Lua, and mistaking one for the other is the mistake this prompt exists
/// to prevent.
///
/// The directory and the branch are on the *right*, because both change constantly and would
/// otherwise push the command you are typing further and further across the screen.
pub fn render_default_left_prompt(last_status: i32, language: &str) -> String {
    let theme = theme::current();
    let depth = theme::depth();
    let bar = theme.prompt.aside.paint(" | ", depth);

    let mut out = theme.prompt.user.paint(&username(), depth);
    out.push_str(&theme.prompt.aside.paint("@", depth));
    out.push_str(&theme.prompt.host.paint(&hostname(), depth));

    // The mode letter. rustyline never redraws a prompt, so oslo repaints this line itself the
    // moment the mode changes — see `repaint` below and its caller in the key handler.
    if let Some(mode) = super::vi::mode() {
        out.push_str(&bar);
        let style = match mode {
            super::vi::Mode::Insert => theme.prompt.ok,
            super::vi::Mode::Normal => theme.prompt.host,
            super::vi::Mode::Replace => theme.prompt.failed,
        };
        out.push_str(&style.paint(mode.name(), depth));
    }

    out.push_str(&bar);
    out.push_str(&theme.prompt.git.paint(language, depth));

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

/// This machine's name, short — everything before the first dot.
///
/// A fully qualified name is most of the prompt's width on a machine that has one, and the part
/// that identifies it to a person is the first label.
fn hostname() -> String {
    nix::unistd::gethostname()
        .ok()
        .and_then(|h| h.into_string().ok())
        .map(|h| h.split('.').next().unwrap_or(&h).to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// Who you are. `$USER` first, then the password database, because `$USER` is what `su` updates
/// and the uid is what it does not.
fn username() -> String {
    env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| {
            nix::unistd::User::from_uid(nix::unistd::getuid())
                .ok()
                .flatten()
                .map(|u| u.name)
        })
        .unwrap_or_else(|| "?".to_string())
}

/// A duration worth mentioning, or `None`.
///
/// Short commands are the overwhelming majority and saying `3ms` after each of them is noise, so
/// nothing is shown below the threshold. The number that *is* shown is the one you would have
/// wanted before you knew you wanted it — which is the whole argument for a duration in a prompt
/// rather than a `time` you have to remember to type.
pub fn notable_duration(elapsed: Duration) -> Option<String> {
    const WORTH_SAYING: Duration = Duration::from_millis(500);
    if elapsed < WORTH_SAYING {
        return None;
    }
    let secs = elapsed.as_secs_f64();
    Some(if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let whole = elapsed.as_secs();
        format!("{}m{:02}s", whole / 60, whole % 60)
    })
}

/// The right prompt oslo draws when a config has not asked for its own.
///
/// It shows what the left prompt cannot: the *number* behind a failing status — the left arrow only
/// goes red — how long the last command took when that is worth saying, and the time. A successful,
/// quick command leaves only the clock, so the line stays quiet until it has something to report.
pub fn render_default_right_prompt(last_status: i32, elapsed: Option<Duration>) -> String {
    let theme = theme::current();
    let depth = theme::depth();
    // The mirror of the left prompt's `❯`, opening the right side the way that one closes the
    // left. It takes the same colour, so the pair reads as one frame around the line you type.
    let arrow = if last_status == 0 {
        theme.prompt.ok
    } else {
        theme.prompt.failed
    };
    let mut parts = vec![arrow.paint("❮", depth)];

    // The status number, which the left arrow's colour cannot carry.
    if last_status != 0 {
        parts.push(
            theme
                .prompt
                .failed
                .paint(&format!("({last_status})"), depth),
        );
    }
    if let Some(took) = elapsed.and_then(notable_duration) {
        parts.push(theme.prompt.aside.paint(&took, depth));
    }
    // The branch sits with the directory, because they answer the same question — *where* you are
    // — and separating them meant reading both ends of the line to know it.
    if let Some(branch) = git_branch() {
        parts.push(theme.prompt.git.paint(&format!("({branch})"), depth));
    }
    // The directory lives here rather than on the left: it is what changes on every `cd`, and on
    // the right it does not push the command you are typing further and further across.
    let pwd = tilde(
        &env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "/".to_string()),
    );
    parts.push(theme.prompt.cwd.paint(&pwd, depth));
    parts.join(&theme.prompt.aside.paint("  ", depth))
}

/// What is currently on the prompt's row, so it can be drawn again without the line editor.
///
/// rustyline hands its prompt over once and never redraws it, and a key handler cannot ask it to —
/// `EventContext` carries a `&dyn Refresher` while `refresh_prompt_and_line` wants `&mut`. So when
/// the vi mode changes there is no way to make rustyline repaint, and a mode letter in the prompt
/// would sit there saying `I` while the cursor said otherwise.
///
/// oslo repaints the row itself instead. The highlighter runs on every line change and knows
/// everything needed — the language, the line, and where the cursor sits — so it leaves a copy
/// here, and [`repaint`] writes it out again with whatever the mode is *now*.
static ROW: std::sync::Mutex<Option<Row>> = std::sync::Mutex::new(None);

#[derive(Clone)]
struct Row {
    /// The language segment, so the prompt can be rebuilt for the right one.
    language: String,
    status: i32,
    /// Cells the prompt itself occupies, so the cursor column can be worked out from a position
    /// within the line.
    prompt_width: usize,
}

/// Record the row, for [`repaint`]. Called by the highlighter on every redraw.
pub fn note_row(language: &str, status: i32, prompt_width: usize) {
    if let Ok(mut slot) = ROW.lock() {
        *slot = Some(Row {
            language: language.to_string(),
            status,
            prompt_width,
        });
    }
}

/// Draw the prompt row again, with the vi mode as it stands now.
///
/// Returns the escapes to write, or empty when there is nothing recorded — the first prompt of a
/// session, before anything has been highlighted.
///
/// **Only the prompt is rewritten — never the line, and nothing is erased.**
///
/// That restraint is the whole of it. The first attempt cleared the row and redrew prompt *and*
/// line from the highlighter's snapshot, which broke the ghost suggestion and the completion
/// dropdown outright: rustyline draws prompt, line, *and hint*, and the snapshot has no hint in
/// it. The row came back without one while rustyline still believed it was there, so every later
/// refresh measured against a row that no longer matched.
///
/// Overwriting just the prompt is safe because a prompt's width does not change with the mode —
/// `I`, `N` and `R` are one cell each — so the line and the hint after it are untouched, and
/// rustyline's idea of the row stays true. `\r` to the start, write, `\r` and forward to wherever
/// the cursor was.
/// `line_cursor` is how many cells into the *line* the cursor sits; the prompt's own width is
/// added here. The caller gets it from the line and byte position the editor hands over.
///
/// **Not the end of the line.** Restoring to the end was the first version's bug: with the cursor
/// anywhere but the end, every mode change dragged it to the right, which looks like the block
/// jumping a slot and makes everything typed afterwards land in the wrong place.
/// Switch the language the prompt shows, answering the one now in force.
///
/// The prompt is the only place the language is written down between keystrokes, so the toggle
/// changes it here and repaints. It used to accept the line to hand control back to the read
/// loop, which cost a row and a fresh prompt every time you changed your mind about what you were
/// typing — and the thing you had already typed had to be carried across by hand.
pub fn toggle_language() -> String {
    let Ok(mut slot) = ROW.lock() else {
        return "sh".to_string();
    };
    let Some(row) = slot.as_mut() else {
        return "sh".to_string();
    };
    row.language = if row.language == "sh" { "lua" } else { "sh" }.to_string();
    row.language.clone()
}

/// The language the prompt is currently showing.
pub fn language() -> Option<String> {
    ROW.lock().ok()?.as_ref().map(|row| row.language.clone())
}

pub fn repaint(line_cursor: usize) -> String {
    let Ok(slot) = ROW.lock() else {
        return String::new();
    };
    let Some(row) = slot.as_ref() else {
        return String::new();
    };
    let left = render_default_left_prompt(row.status, &row.language);
    let cursor = row.prompt_width + line_cursor;
    let mut out = format!("\r{left}");
    out.push('\r');
    if cursor > 0 {
        out.push_str(&format!("\x1b[{cursor}C"));
    }
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
    // Move right, draw, move back — **not** save/restore.
    //
    // `\x1b7`/`\x1b8` (DECSC/DECRC) has exactly one slot per terminal, and it is shared with
    // everything else drawing on it: oslo's own completion dropdown uses it, and a multiplexer
    // hosting the session may too. Whoever saves last wins, so a restore can land wherever
    // somebody else's save left it — which shows up as a right prompt that jumps, duplicates, or
    // strands debris a row up. Relative motion has no shared state to lose.
    //
    // The gap is `cols - right_w - used`: from the cursor's column to the first cell of the right
    // prompt. Coming back is that gap plus the text just drawn.
    let gap = cols - right_w - used;
    // The last cell written is the final column, which leaves most terminals in a deferred-wrap
    // state. `\r` settles that — the cursor is unambiguously at column 1 afterwards — and the
    // forward move puts it back, rather than trusting the terminal to agree about where it was.
    let home = used;
    if home == 0 {
        format!("\x1b[{gap}C{right}\r")
    } else {
        format!("\x1b[{gap}C{right}\r\x1b[{home}C")
    }
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
        // From column 11 to column 76 is 65 cells forward; `12:34` then ends on column 80.
        assert!(escape.starts_with("\x1b[65C"), "{escape:?}");
        assert!(escape.contains("12:34"), "{escape:?}");
        // Back to where it started, via column 1 — see the comment on the deferred wrap.
        assert!(escape.ends_with("\r\x1b[10C"), "{escape:?}");

        // **No save/restore.** There is one DECSC slot per terminal and it is shared with the
        // dropdown and with any multiplexer hosting the session; a restore could land wherever
        // somebody else's save left it, which is what made the right prompt jump and duplicate.
        assert!(!escape.contains("\x1b7"), "{escape:?}");
        assert!(!escape.contains("\x1b8"), "{escape:?}");

        // A prompt at column 1 needs no move back, only the `\r`.
        let at_start = right_prompt_escape("12:34", 0, 80);
        assert!(at_start.ends_with("\r"), "{at_start:?}");
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
        // The leading forward move, which is what carries the position.
        let moved = |s: &str| s.split('C').next().map(str::to_string);
        assert_eq!(moved(&plain), moved(&painted));
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

#[cfg(test)]
mod right_prompt_tests {
    use super::{notable_duration, render_default_right_prompt};
    use std::time::Duration;

    /// Short commands are the overwhelming majority; saying `3ms` after each is noise.
    #[test]
    fn only_a_duration_worth_saying_is_said() {
        assert_eq!(notable_duration(Duration::from_millis(3)), None);
        assert_eq!(notable_duration(Duration::from_millis(499)), None);
        assert_eq!(
            notable_duration(Duration::from_millis(1300)).as_deref(),
            Some("1.3s")
        );
        assert_eq!(
            notable_duration(Duration::from_secs(75)).as_deref(),
            Some("1m15s")
        );
    }

    /// A quick success leaves only the clock — the line stays quiet until it has something to
    /// report. A failure shows the *number*, which the left prompt's arrow cannot.
    #[test]
    fn the_right_prompt_reports_only_what_is_worth_reporting() {
        let quiet = plain(&render_default_right_prompt(
            0,
            Some(Duration::from_millis(3)),
        ));
        assert!(
            quiet.starts_with('❮'),
            "the mirror of the left arrow: {quiet:?}"
        );
        // No *status* on a success. Checked as "a paren followed by a digit" rather than as any
        // paren, because the git branch lives here too and `(develop)` is not an exit code.
        assert!(
            !quiet
                .match_indices('(')
                .any(|(i, _)| quiet[i + 1..].starts_with(|c: char| c.is_ascii_digit())),
            "no status on a success: {quiet:?}"
        );
        assert!(
            !quiet.contains("ms") && !quiet.contains("0.0s"),
            "no duration for a quick command: {quiet:?}"
        );

        let failed = plain(&render_default_right_prompt(7, None));
        assert!(failed.contains("(7)"), "{failed:?}");

        let slow = plain(&render_default_right_prompt(
            0,
            Some(Duration::from_secs(3)),
        ));
        assert!(slow.contains("3.0s"), "{slow:?}");
    }

    /// Strip the styling, leaving what is on screen.
    fn plain(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}
