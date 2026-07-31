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

mod layout;
mod render;
mod width;

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
}

impl CompletionCandidate {
    pub fn new(display: String, replacement: String, description: Option<String>) -> Self {
        Self {
            display,
            replacement,
            description,
            kind: None,
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

        let stdin = io::stdin();
        let orig_termios = tcgetattr(&stdin).ok()?;
        let mut raw_termios = orig_termios.clone();
        raw_termios.local_flags.remove(LocalFlags::ICANON);
        raw_termios.local_flags.remove(LocalFlags::ECHO);
        let _ = tcsetattr(&stdin, SetArg::TCSANOW, &raw_termios);

        let mut stdout = io::stdout();

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
            let _ = write!(stdout, "{}", rendered);
            let _ = stdout.flush();

            let mut buf = [0u8; 3];
            let n = io::stdin().read(&mut buf).unwrap_or(0);

            // Walk back over the *physical* rows the dropdown occupied, then erase below the
            // prompt. `num_lines` already accounts for any wrapping.
            let _ = write!(stdout, "\x1b[{}A\r\x1b[J", num_lines);
            let _ = stdout.flush();

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

        let _ = tcsetattr(&stdin, SetArg::TCSANOW, &orig_termios);
        selected
    }
}
