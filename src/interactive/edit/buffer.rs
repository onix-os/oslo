//! The line being typed: text, cursor, and every edit that can be made to it.
//!
//! Pure. Nothing here reads a key, measures a terminal or writes a byte — an edit is a method on a
//! `Buffer` and its result is the `Buffer`. That is what makes the editor testable without a
//! terminal, which is the whole reason oslo can replace a line editor at all: the parts that are
//! hard to get right are the parts that can be checked in a unit test.
//!
//! # Characters, not bytes
//!
//! The cursor is an index into a `Vec<char>`. A shell line is short, so the copying a `Vec<char>`
//! costs is irrelevant beside never having to ask whether an index is on a character boundary —
//! and a cursor that can land mid-character is how editors corrupt UTF-8. Display *width* is a
//! separate question, answered in [`super::layout`], because one character is not one cell.
//!
//! # Two kinds of word, on purpose
//!
//! readline has both and scripts' muscle memory depends on the difference:
//!
//! * `M-b` / `M-f` / `M-DEL` move and kill over **alphanumeric** runs, so `foo-bar` is two words.
//! * `C-w` kills back over a **whitespace**-delimited run, so `foo-bar` is one word — which is
//!   what makes it the key for taking back a path you just typed.
//!
//! Collapsing them into one would break one of the two habits, and both are decades old.

/// What kind of character this is, for word motions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    /// A letter or digit — readline's notion of a word character.
    Word,
    /// Punctuation: `-`, `/`, `.`, and everything else.
    Other,
}

fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Other
    }
}

/// The line under the cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Buffer {
    chars: Vec<char>,
    /// Index into `chars`, from 0 to `chars.len()` inclusive — the end is a valid cursor.
    cursor: usize,
    /// The last killed text, for `C-y`. One entry rather than readline's ring: the ring's
    /// `M-y` rotate is reached by a vanishing fraction of users, and an unused ring is state
    /// that can still be wrong.
    kill: Vec<char>,
}

impl Buffer {
    pub fn new() -> Buffer {
        Buffer::default()
    }

    /// A buffer holding `text` with the cursor at the end, as reopening a line does.
    pub fn from_text(text: &str) -> Buffer {
        let chars: Vec<char> = text.chars().collect();
        Buffer {
            cursor: chars.len(),
            chars,
            kill: Vec::new(),
        }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// The text before the cursor, which is what a completion or a hint is computed from.
    pub fn before_cursor(&self) -> String {
        self.chars[..self.cursor].iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Replace everything, putting the cursor at `cursor` (clamped).
    ///
    /// Used when a history entry, a finder choice or a key handler supplies a whole new line.
    pub fn set(&mut self, text: &str, cursor: usize) {
        self.chars = text.chars().collect();
        self.cursor = cursor.min(self.chars.len());
    }

    pub fn insert(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Insert a run at once — a paste, or an accepted completion.
    ///
    /// One call rather than a loop of [`Self::insert`] because a paste of a thousand characters
    /// should be one shift of the tail, not a thousand.
    pub fn insert_str(&mut self, text: &str) {
        let incoming: Vec<char> = text.chars().collect();
        let at = self.cursor;
        self.chars.splice(at..at, incoming.iter().copied());
        self.cursor += incoming.len();
    }

    /// Delete the character before the cursor. Answers whether there was one.
    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.chars.remove(self.cursor);
        true
    }

    /// Delete the character under the cursor.
    pub fn delete(&mut self) -> bool {
        if self.cursor >= self.chars.len() {
            return false;
        }
        self.chars.remove(self.cursor);
        true
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.chars.len() {
            self.cursor += 1;
        }
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.chars.len();
    }

    /// Where `M-b` lands: back over any run of non-word characters, then over the word itself.
    fn word_start(&self) -> usize {
        let mut at = self.cursor;
        while at > 0 && class(self.chars[at - 1]) != Class::Word {
            at -= 1;
        }
        while at > 0 && class(self.chars[at - 1]) == Class::Word {
            at -= 1;
        }
        at
    }

    /// Where `M-f` lands: forward over non-word characters, then over the word.
    fn word_end(&self) -> usize {
        let mut at = self.cursor;
        let len = self.chars.len();
        while at < len && class(self.chars[at]) != Class::Word {
            at += 1;
        }
        while at < len && class(self.chars[at]) == Class::Word {
            at += 1;
        }
        at
    }

    /// Where `C-w` reaches: back over whitespace, then over everything that is not whitespace.
    ///
    /// The difference from [`Self::word_start`] is the whole point — `C-w` after typing
    /// `/usr/local/bin` takes all of it, and `M-DEL` takes only `bin`.
    fn space_word_start(&self) -> usize {
        let mut at = self.cursor;
        while at > 0 && class(self.chars[at - 1]) == Class::Space {
            at -= 1;
        }
        while at > 0 && class(self.chars[at - 1]) != Class::Space {
            at -= 1;
        }
        at
    }

    pub fn move_word_left(&mut self) {
        self.cursor = self.word_start();
    }

    pub fn move_word_right(&mut self) {
        self.cursor = self.word_end();
    }

    /// Remove `from..to`, remembering it as the kill.
    fn kill_range(&mut self, from: usize, to: usize) -> bool {
        if from >= to {
            return false;
        }
        self.kill = self.chars[from..to].to_vec();
        self.chars.drain(from..to);
        self.cursor = from;
        true
    }

    /// `C-k` — from the cursor to the end of the line.
    pub fn kill_to_end(&mut self) -> bool {
        let end = self.chars.len();
        self.kill_range(self.cursor, end)
    }

    /// `C-u` — from the start of the line to the cursor.
    ///
    /// readline's `unix-line-discard`, which is *to the cursor*, not the whole line. bash users
    /// press it mid-line to drop a bad prefix and expect the tail to survive.
    pub fn kill_to_start(&mut self) -> bool {
        self.kill_range(0, self.cursor)
    }

    /// `M-DEL` — the alphanumeric word before the cursor.
    pub fn kill_word_left(&mut self) -> bool {
        let from = self.word_start();
        self.kill_range(from, self.cursor)
    }

    /// `M-d` — the alphanumeric word after the cursor.
    pub fn kill_word_right(&mut self) -> bool {
        let to = self.word_end();
        self.kill_range(self.cursor, to)
    }

    /// `C-w` — the whitespace-delimited word before the cursor.
    pub fn kill_space_word_left(&mut self) -> bool {
        let from = self.space_word_start();
        self.kill_range(from, self.cursor)
    }

    /// `C-y` — put the last killed text back at the cursor.
    pub fn yank(&mut self) -> bool {
        if self.kill.is_empty() {
            return false;
        }
        let text: String = self.kill.iter().collect();
        self.insert_str(&text);
        true
    }

    /// `C-t` — swap the two characters around the cursor and step past them.
    ///
    /// At the end of the line it swaps the last two, which is what readline does and what the
    /// typo it exists for actually needs.
    pub fn transpose(&mut self) -> bool {
        let len = self.chars.len();
        if len < 2 {
            return false;
        }
        let right = if self.cursor >= len {
            len - 1
        } else {
            self.cursor
        };
        if right == 0 {
            return false;
        }
        self.chars.swap(right - 1, right);
        self.cursor = (right + 1).min(len);
        true
    }

    /// Empty the line, keeping the kill.
    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
    }

    /// Change the case of the word from the cursor, and step over it — `M-u`, `M-l`, `M-c`.
    pub fn case_word(&mut self, how: Case) -> bool {
        let to = self.word_end();
        if to <= self.cursor {
            return false;
        }
        let mut first = true;
        for i in self.cursor..to {
            let c = self.chars[i];
            self.chars[i] = match how {
                Case::Upper => c.to_uppercase().next().unwrap_or(c),
                Case::Lower => c.to_lowercase().next().unwrap_or(c),
                // Only the first *word* character is capitalised; leading punctuation is skipped
                // over rather than counting as the letter that gets the capital.
                Case::Title if first && class(c) == Class::Word => {
                    first = false;
                    c.to_uppercase().next().unwrap_or(c)
                }
                Case::Title => {
                    if class(c) == Class::Word {
                        first = false;
                    }
                    c.to_lowercase().next().unwrap_or(c)
                }
            };
        }
        self.cursor = to;
        true
    }
}

/// Which way [`Buffer::case_word`] converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    Upper,
    Lower,
    Title,
}

#[cfg(test)]
#[path = "buffer/tests.rs"]
mod tests;
