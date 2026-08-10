//! Taking a proposal into the line.
//!
//! Two of them, and the difference between them is the whole of this file. A ghost suggestion
//! *continues* what was typed, so it is appended and the text you wrote survives. A correction says
//! what the line should have been instead, so it replaces. One key accepts either — they are drawn
//! in the same place and never at once — which only works because each says for itself whether it
//! had anything to give.

use super::{Assist, Session, first_word};

impl Session {
    /// Take the ghost suggestion into the line — all of it, or one word.
    ///
    /// The suggestion is *what would be drawn now*, asked for again rather than remembered from
    /// the last frame — a remembered one can be stale by exactly the keystroke that accepted it.
    ///
    /// `false` when there was nothing to take, so a key that also means something else can fall
    /// through to that meaning.
    pub(super) fn take_hint(&mut self, whole: bool, assist: &mut dyn Assist) -> bool {
        let line = self.buffer.text();
        let Some(hint) = assist.hint_text(&line, self.buffer.cursor()) else {
            return false;
        };
        let take = if whole { hint } else { first_word(&hint) };
        if take.is_empty() {
            return false;
        }
        self.buffer.move_end();
        self.buffer.insert_str(&take);
        true
    }

    /// Take the correction into the line, replacing what was typed.
    ///
    /// **A replacement, not an append**, which is the one way this differs from [`Self::take_hint`]
    /// — and the reason the two cannot share a path. Only offered when the whole line is a
    /// near-miss, so what is discarded is the thing the user is being told is wrong.
    pub(super) fn take_repair(&mut self, assist: &mut dyn Assist) -> bool {
        let line = self.buffer.text();
        let Some(fixed) = assist.repair_text(&line, self.buffer.cursor()) else {
            return false;
        };
        if fixed.is_empty() || fixed == line {
            return false;
        }
        self.buffer.clear();
        self.buffer.insert_str(&fixed);
        true
    }
}
