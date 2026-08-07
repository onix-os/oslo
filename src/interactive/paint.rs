//! Drawing a block of rows under the cursor and taking it away again.
//!
//! Every widget that appears *below* where the cursor already is — the completion dropdown, and
//! everything in [`crate::interactive::ask`] that shows a list — needs the same three moves, and
//! all three are easy to get wrong in the same way.
//!
//! # The bug this exists to prevent
//!
//! The obvious way to draw eight rows below the prompt is to print them and then move up eight.
//! It works everywhere except the bottom of the screen, and there it eats the prompt: printing
//! newlines on the last row makes the terminal **scroll**, so the cursor ends up fewer rows down
//! than were printed, and `ESC [ 8 A` lands *above* where you started. The next erase then removes
//! a line that was never yours. Holding an arrow key makes it worse every frame, which is what it
//! looks like from the outside: the prompt marching up the screen and disappearing.
//!
//! The fix is to make the scroll happen *before* anything is drawn. `Panel::reserve` prints the
//! newlines first, while the cursor is still ours to account for, and comes back up. After that
//! the rows exist, and every later frame is a redraw in place with no scrolling and no arithmetic
//! that can drift.
//!
//! # No `ESC 7` / `ESC 8`
//!
//! There is one cursor-save slot per terminal and it is shared with everything else drawing on it,
//! including whatever multiplexer is hosting the session. A restore lands wherever somebody else's
//! save left the cursor. Every move here is relative, which has no shared state to lose.

/// A block of rows owned by one widget, and the cursor position it draws from.
#[derive(Debug, Default)]
pub struct Panel {
    /// How many rows have been reserved. Grows, never shrinks: a frame shorter than the last one
    /// still owns the rows the taller one took, and `ESC [ J` clears the difference.
    reserved: usize,
    /// The column the cursor sat in when the panel opened, so coming back is a move rather than a
    /// restore.
    column: usize,
}

/// Begin an atomic update, so the terminal cannot show a half-drawn frame.
///
/// DEC mode 2026: a terminal that understands it buffers everything until the matching end and
/// presents the result in one go; one that does not ignores both, so this costs nothing anywhere.
///
/// It matters wherever a frame is more than one write — reserve rows, move, erase, redraw — which
/// is every widget that draws through this module. Without it the terminal is free to render
/// halfway through a rewrite, and that is what tearing *is*: on a list it reads as the rows
/// flickering or jumping.
///
/// Here rather than in one widget because both things that draw whole frames need it, and having
/// only one of them do it is how the finder came to look steady while `ui filter` did not.
pub const SYNC_BEGIN: &str = "\x1b[?2026h";
pub const SYNC_END: &str = "\x1b[?2026l";

impl Panel {
    /// A panel drawn from `column`.
    pub fn at(column: usize) -> Panel {
        Panel {
            reserved: 0,
            column,
        }
    }

    /// How many rows this panel currently owns.
    pub fn reserved(&self) -> usize {
        self.reserved
    }

    /// The escapes that draw `rows` rows of `frame` and leave the cursor where it started.
    ///
    /// `frame` must begin each row with `\r\n`, so that after drawing, the cursor is `rows` below
    /// the start and in column 1 — which is what makes the walk back up exact.
    pub fn draw(&mut self, frame: &str, rows: usize) -> String {
        let mut out = self.reserve(rows);
        out.push_str(frame);
        // Everything below the last drawn row belongs to this panel — the rows were reserved above
        // and are cleared together — so erasing to the end of the screen is safe, and it is what
        // stops a shorter frame from leaving the tail of a taller one on screen.
        out.push_str("\x1b[J");
        if rows > 0 {
            out.push_str(&format!("\x1b[{rows}A"));
        }
        out.push('\r');
        if self.column > 0 {
            out.push_str(&format!("\x1b[{}C", self.column));
        }
        out
    }

    /// Make room for `wanted` rows below the cursor, scrolling now rather than mid-draw.
    fn reserve(&mut self, wanted: usize) -> String {
        if wanted <= self.reserved {
            return String::new();
        }
        let extra = wanted - self.reserved;
        self.reserved = wanted;
        format!("{}\x1b[{extra}A", "\n".repeat(extra))
    }

    /// Erase everything this panel drew, leaving the cursor where it started.
    ///
    /// `ESC [ B` rather than a newline: the rows already exist, and a newline at the bottom of the
    /// screen would scroll again — the whole class of bug this module avoids.
    pub fn close(&mut self) -> String {
        if self.reserved == 0 {
            return String::new();
        }
        self.reserved = 0;
        let mut out = String::from("\x1b[B\r\x1b[J\x1b[A\r");
        if self.column > 0 {
            out.push_str(&format!("\x1b[{}C", self.column));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first frame reserves its rows *before* drawing them. That ordering is the whole fix:
    /// the newlines come first, so any scrolling happens while the cursor is still accounted for.
    #[test]
    fn the_first_frame_reserves_before_it_draws() {
        let mut panel = Panel::at(0);
        let out = panel.draw("\r\na\r\nb", 2);
        let newlines = out.find("\n\n").expect("rows are reserved first");
        let content = out.find('a').expect("content is drawn");
        assert!(newlines < content, "reserved after drawing: {out:?}");
        assert!(out.starts_with("\n\n\x1b[2A"), "{out:?}");
    }

    /// Rows are reserved once. A second frame of the same height reserves nothing more, or every
    /// keystroke would push the screen down by another block.
    #[test]
    fn rows_are_reserved_once() {
        let mut panel = Panel::at(0);
        panel.draw("\r\na\r\nb", 2);
        let second = panel.draw("\r\nc\r\nd", 2);
        assert!(!second.starts_with('\n'), "reserved twice: {second:?}");
        assert_eq!(panel.reserved(), 2);
    }

    /// A taller frame reserves only the difference.
    #[test]
    fn growing_reserves_only_the_extra_rows() {
        let mut panel = Panel::at(0);
        panel.draw("\r\na", 1);
        let taller = panel.draw("\r\na\r\nb\r\nc", 3);
        assert!(taller.starts_with("\n\n\x1b[2A"), "{taller:?}");
        assert_eq!(panel.reserved(), 3);
    }

    /// A shorter frame keeps the rows it had. The `ESC [ J` clears the difference, which is what
    /// stops the previous, taller frame's last rows from staying on screen underneath.
    #[test]
    fn shrinking_keeps_the_rows_and_clears_below() {
        let mut panel = Panel::at(0);
        panel.draw("\r\na\r\nb\r\nc", 3);
        let shorter = panel.draw("\r\na", 1);
        assert!(!shorter.starts_with('\n'), "{shorter:?}");
        assert!(
            shorter.contains("\x1b[J"),
            "nothing clears below: {shorter:?}"
        );
        assert_eq!(panel.reserved(), 3, "rows are kept, not given back");
    }

    /// The cursor comes back to the column it started in, by moving — never by restoring.
    #[test]
    fn the_cursor_returns_to_its_column() {
        let mut panel = Panel::at(12);
        let out = panel.draw("\r\na", 1);
        assert!(out.ends_with("\r\x1b[12C"), "{out:?}");
        // Column 0 needs no move at all, and emitting `ESC [ 0 C` would be a move of one on some
        // terminals.
        let mut origin = Panel::at(0);
        let out = origin.draw("\r\na", 1);
        assert!(out.ends_with('\r'), "{out:?}");
    }

    /// Nothing in here may use the shared cursor-save slot; a multiplexer owns it too.
    #[test]
    fn the_shared_save_slot_is_never_touched() {
        let mut panel = Panel::at(5);
        let drawn = panel.draw("\r\na\r\nb", 2);
        let closed = panel.close();
        for out in [&drawn, &closed] {
            assert!(!out.contains("\x1b7"), "saves the cursor: {out:?}");
            assert!(!out.contains("\x1b8"), "restores the cursor: {out:?}");
        }
    }

    #[test]
    fn closing_erases_and_forgets() {
        let mut panel = Panel::at(3);
        panel.draw("\r\na\r\nb", 2);
        let closed = panel.close();
        assert!(closed.contains("\x1b[J"), "{closed:?}");
        assert!(closed.ends_with("\r\x1b[3C"), "{closed:?}");
        assert_eq!(panel.reserved(), 0);
        // Closing twice draws nothing the second time.
        assert_eq!(panel.close(), "");
    }

    /// A panel that never drew has nothing to erase.
    #[test]
    fn closing_an_unused_panel_is_nothing() {
        assert_eq!(Panel::at(4).close(), "");
    }
}
