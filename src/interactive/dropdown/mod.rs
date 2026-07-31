//! The completion dropdown: candidates, layout, rendering, and the raw-mode selection loop.
//!
//! Everything here is measured in *terminal cells*, not bytes and not `char`s. A box drawn from
//! byte lengths overflows as soon as a label carries an emoji icon, and a box drawn without
//! asking how wide the terminal is overflows as soon as the cwd is deep. An overflowing row
//! wraps; a wrapped row makes the "move the cursor back up" sequence count too few rows; and the
//! redraw then paints over the prompt and the scrollback above it (R9.4).
//!
//! The three pieces below keep that from happening:
//!
//! * `width` — cells per string, wraps per row, and the terminal's own size via `TIOCGWINSZ`;
//! * `layout` — column widths clamped so no row can exceed the screen;
//! * `render` — the escapes, plus the physical row count to walk back up.
//!
//! [`render_vertical_dropdown_at_width`] takes the column count as an argument rather than
//! querying it, so the whole layout is testable at 80 columns with no terminal attached.

mod columns;
mod layout;
mod render;
mod width;

pub use columns::{
    Facts, Provider, builtin_columns, columns_for, facts_for, human_age, human_mode, human_size,
    set_provider,
};
pub use layout::{DropdownLayout, compute_layout};
pub use render::{MAX_ROWS, render_vertical_dropdown, render_vertical_dropdown_at_width};
pub use width::{
    FALLBACK_COLS, display_width, pad_to_width, physical_rows, terminal_cols, truncate_to_width,
    visible_len,
};

use nix::sys::termios::{LocalFlags, SetArg, tcgetattr, tcsetattr};
use std::io::{self, Read, Write};

#[derive(Debug, Clone)]
pub struct CompletionCandidate {
    pub display: String,
    pub replacement: String,
    pub description: Option<String>,
    pub kind: Option<String>,
    /// The file this candidate names, when it names one.
    ///
    /// Carried rather than reconstructed: `replacement` is *quoted*, so working back to a path
    /// would mean unquoting it — and getting that wrong turns a `stat` of `My File.txt` into a
    /// `stat` of `My\ File.txt`, which simply is not there. The completer already knows the path
    /// it built, so it hands it over.
    pub path: Option<String>,
    /// A fact the completer already knew and the renderer would have to guess.
    ///
    /// What an alias expands to, mainly. It is captured here rather than looked up at render time
    /// because the renderer has no environment: it is handed a list and a width, and reaching back
    /// into the shell from inside a draw would put a lock on the frame path.
    pub detail: Option<String>,
}

impl CompletionCandidate {
    pub fn new(display: String, replacement: String, description: Option<String>) -> Self {
        Self {
            display,
            replacement,
            description,
            kind: None,
            path: None,
            detail: None,
        }
    }

    /// The word the kind badge spells, or `None` for a candidate with no kind.
    ///
    /// Replaces an emoji icon per kind. Two reasons it went: an emoji is two cells wide in some
    /// terminals and one in others, so a column of them cannot be laid out reliably; and a glyph
    /// has to be learned, where ` builtin ` is already the word. The badge is drawn as a coloured
    /// pill, which is what carries the "this is a kind" reading that the icon was there for.
    pub fn badge(&self) -> Option<&str> {
        match self.kind.as_deref() {
            Some("dir") | Some("directory") => Some("dir"),
            Some("file") => Some("file"),
            Some("builtin") => Some("builtin"),
            Some("command") => Some("command"),
            Some("variable") => Some("variable"),
            Some("history") => Some("history"),
            Some("alias") => Some("alias"),
            Some("flag") | Some("option") => Some("option"),
            Some("subcommand") => Some("subcmd"),
            Some("function") => Some("func"),
            // A kind nothing has claimed is still a kind; showing it is how the next one gets
            // noticed rather than silently drawn as blank.
            Some(other) if !other.is_empty() => Some(other),
            _ => None,
        }
    }
}

pub struct DropdownMenu {
    pub candidates: Vec<CompletionCandidate>,
    pub selected_index: usize,
    pub max_visible: usize,
    pub indent_cols: usize,
}

impl DropdownMenu {
    pub fn new(candidates: Vec<CompletionCandidate>, indent_cols: usize) -> Self {
        Self {
            candidates,
            selected_index: 0,
            max_visible: MAX_ROWS,
            indent_cols,
        }
    }

    pub fn select_interactive(
        candidates: Vec<CompletionCandidate>,
        indent_cols: usize,
        typed: &str,
    ) -> Option<CompletionCandidate> {
        if candidates.is_empty() {
            return None;
        }
        if candidates.len() == 1 {
            return Some(candidates[0].clone());
        }

        let mut menu = Self::new(candidates, indent_cols);
        // `oslo.completion.max_rows`.
        menu.max_visible = menu
            .max_visible
            .min(crate::interactive::settings::current().completion.max_rows);

        let stdin = io::stdin();
        let orig_termios = tcgetattr(&stdin).ok()?;
        let mut raw_termios = orig_termios.clone();
        raw_termios.local_flags.remove(LocalFlags::ICANON);
        raw_termios.local_flags.remove(LocalFlags::ECHO);
        let _ = tcsetattr(&stdin, SetArg::TCSANOW, &raw_termios);

        let mut stdout = io::stdout();
        // Rows currently reserved below the prompt. See the comment in the loop.
        let mut reserved = 0usize;

        let selected = loop {
            // The width is re-queried every frame: the terminal can be resized while the menu is
            // open, and a stale width is exactly the state that overwrites the prompt.
            let (rendered, num_lines) = render_vertical_dropdown_at_width(
                &menu.candidates,
                menu.selected_index,
                menu.max_visible,
                menu.indent_cols,
                terminal_cols(),
                typed,
            );
            // **Reserve the rows before drawing into them.** Drawing first and walking back up
            // afterwards is what ate the prompt: near the bottom of the screen the newlines make
            // the terminal *scroll*, so the cursor ends up fewer rows down than were printed, and
            // moving up by the full count lands above the prompt — where the erase then removes
            // it. Pressing the arrow keys made it worse each frame.
            //
            // Printing the newlines first makes any scroll happen while the cursor is still ours
            // to account for: after moving back up the same number, the prompt is wherever the
            // scroll left it, and every frame after this one is a redraw in place.
            let _ = write!(stdout, "{}", reserve_rows(num_lines, &mut reserved));
            let _ = write!(stdout, "{}", draw_below(&rendered));
            let _ = stdout.flush();

            let mut buf = [0u8; 3];
            let n = io::stdin().read(&mut buf).unwrap_or(0);

            if n == 0 {
                break None;
            }

            if n == 1 {
                match buf[0] {
                    13 | 10 | 32 => {
                        // Enter or Space
                        break Some(menu.candidates[menu.selected_index].clone());
                    }
                    9 => {
                        // Tab cycles down
                        menu.selected_index = (menu.selected_index + 1) % menu.candidates.len();
                    }
                    27 => {
                        // Esc
                        break None;
                    }
                    _ => break None,
                }
            } else if n == 3 && buf[0] == 27 && buf[1] == 91 {
                match buf[2] {
                    65 => {
                        // Up Arrow
                        if menu.selected_index > 0 {
                            menu.selected_index -= 1;
                        } else {
                            menu.selected_index = menu.candidates.len() - 1;
                        }
                    }
                    66 => {
                        // Down Arrow
                        menu.selected_index = (menu.selected_index + 1) % menu.candidates.len();
                    }
                    _ => break None,
                }
            } else {
                break None;
            }
        };

        // Erase what was drawn, from one row below the prompt to the end of the screen. `\x1b[B`
        // rather than a newline: the reserved rows already exist, and a newline at the bottom of
        // the screen would scroll again.
        let _ = write!(stdout, "{}", erase_below(reserved));
        let _ = stdout.flush();

        let _ = tcsetattr(&stdin, SetArg::TCSANOW, &orig_termios);
        selected
    }
}

/// Make room below the prompt for `wanted` rows, given how many are already reserved.
///
/// **This is the fix for the menu eating the prompt.** The old loop drew first and walked back up
/// afterwards; near the bottom of the screen the newlines make the terminal *scroll*, so the
/// cursor ends up fewer rows down than were printed and moving up by the full count lands above
/// the prompt — where the erase then removes it. Holding an arrow key made it worse every frame.
///
/// Printing the newlines *first* makes any scroll happen while the cursor is still ours to
/// account for: moving back up the same number returns to the prompt wherever the scroll left it,
/// and every frame after that is a redraw into rows that already exist.
fn reserve_rows(wanted: usize, reserved: &mut usize) -> String {
    if wanted <= *reserved {
        return String::new();
    }
    let extra = wanted - *reserved;
    *reserved = wanted;
    format!("{}\x1b[{extra}A", "\n".repeat(extra))
}

/// Draw `rendered` below the cursor and come back to it. No arithmetic to get wrong.
fn draw_below(rendered: &str) -> String {
    format!("\x1b7{rendered}\x1b8")
}

/// Erase everything from one row below the prompt to the end of the screen.
///
/// `\x1b[B` rather than a newline: the reserved rows already exist, and a newline at the bottom of
/// the screen would scroll again — which is the whole class of bug this avoids.
fn erase_below(reserved: usize) -> String {
    if reserved == 0 {
        return String::new();
    }
    "\x1b7\x1b[B\r\x1b[J\x1b8".to_string()
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    /// The rows are reserved before anything is drawn into them, and only the shortfall is added.
    #[test]
    fn rows_are_reserved_once_and_only_grown() {
        let mut reserved = 0;
        // Four rows: print four newlines, then come back up four.
        assert_eq!(reserve_rows(4, &mut reserved), "\n\n\n\n\x1b[4A");
        assert_eq!(reserved, 4);
        // The same height again asks for nothing: the rows are already there.
        assert_eq!(reserve_rows(4, &mut reserved), "");
        // A taller menu adds only the difference.
        assert_eq!(reserve_rows(6, &mut reserved), "\n\n\x1b[2A");
        assert_eq!(reserved, 6);
        // A shorter one keeps what it has rather than giving rows back, which would scroll again.
        assert_eq!(reserve_rows(2, &mut reserved), "");
        assert_eq!(reserved, 6);
    }

    /// Every frame is drawn between a save and a restore, so the cursor ends where it started and
    /// no count has to be right for the prompt to survive.
    #[test]
    fn a_frame_saves_and_restores_rather_than_counting_rows_back() {
        let frame = draw_below("ROWS");
        assert!(frame.starts_with("\x1b7"), "{frame:?}");
        assert!(frame.ends_with("\x1b8"), "{frame:?}");
        assert!(frame.contains("ROWS"));
        // The old approach walked back up by a row count. Nothing does that any more.
        assert!(
            !frame.contains("A"),
            "a cursor-up count survived: {frame:?}"
        );
    }

    #[test]
    fn erasing_moves_down_a_row_rather_than_printing_a_newline() {
        let erase = erase_below(3);
        assert!(erase.contains("\x1b[B"), "{erase:?}");
        assert!(!erase.contains('\n'), "a newline would scroll: {erase:?}");
        assert!(erase.contains("\x1b[J"), "{erase:?}");
        // Nothing was drawn, so there is nothing to erase.
        assert_eq!(erase_below(0), "");
    }
}
