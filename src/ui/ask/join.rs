//! `ui join` — put blocks of text beside or above each other.
//!
//! The companion to [`super::style`]: that one draws a box, this one puts two boxes side by side.
//! Together they are how a script lays anything out without counting columns by hand.
//!
//! # Why it is not `paste`
//!
//! `paste` joins *lines* and knows nothing about width, so two blocks of different widths come out
//! ragged and a block containing colour comes out misaligned — the escapes count as characters to
//! everything in coreutils. Here every line is padded to its block's real printed width, so a
//! coloured box joined to a plain one still lines up.

use crate::ui::dropdown::width::pad_to_width;
use crate::ui::prompt::printed_width;

/// Which edge the shorter block is lined up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Top for a horizontal join, left for a vertical one.
    Start,
    Middle,
    /// Bottom for a horizontal join, right for a vertical one.
    End,
}

impl Align {
    pub fn parse(name: &str) -> Option<Align> {
        Some(match name {
            "top" | "left" | "start" => Align::Start,
            "center" | "centre" | "middle" => Align::Middle,
            "bottom" | "right" | "end" => Align::End,
            _ => return None,
        })
    }

    /// How many of `spare` go before the block.
    fn before(self, spare: usize) -> usize {
        match self {
            Align::Start => 0,
            Align::Middle => spare / 2,
            Align::End => spare,
        }
    }
}

/// Put `blocks` side by side.
pub fn horizontal(blocks: &[String], align: Align) -> String {
    let split: Vec<Vec<&str>> = blocks.iter().map(|b| b.split('\n').collect()).collect();
    let tallest = split.iter().map(|b| b.len()).max().unwrap_or(0);
    // Each block's own width, measured in cells so colour does not throw it off.
    let widths: Vec<usize> = split
        .iter()
        .map(|b| b.iter().map(|l| printed_width(l)).max().unwrap_or(0))
        .collect();

    let mut rows = vec![String::new(); tallest];
    for (block, width) in split.iter().zip(&widths) {
        let spare = tallest - block.len();
        let above = align.before(spare);
        for (row, line) in rows.iter_mut().enumerate() {
            let text = if row >= above && row - above < block.len() {
                block[row - above]
            } else {
                ""
            };
            line.push_str(&pad_to_width(text, *width));
        }
    }
    // Trailing padding on the last block is invisible and only makes the output wider than it
    // needs to be — which matters when the result is piped into something that wraps.
    rows.iter()
        .map(|row| row.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Stack `blocks`, each aligned within the widest.
pub fn vertical(blocks: &[String], align: Align) -> String {
    let widest = blocks
        .iter()
        .flat_map(|b| b.split('\n'))
        .map(printed_width)
        .max()
        .unwrap_or(0);
    let mut rows = Vec::new();
    for block in blocks {
        for line in block.split('\n') {
            let spare = widest.saturating_sub(printed_width(line));
            let before = align.before(spare);
            rows.push(format!("{}{}", " ".repeat(before), line));
        }
    }
    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(text: &str) -> String {
        text.to_string()
    }

    #[test]
    fn blocks_sit_side_by_side() {
        let out = horizontal(&[block("a\nb"), block("1\n2")], Align::Start);
        assert_eq!(out, "a1\nb2");
    }

    /// A shorter block is padded, not truncated, and where the padding goes is the alignment.
    #[test]
    fn a_shorter_block_is_aligned_not_cut() {
        let tall = block("1\n2\n3");
        assert_eq!(
            horizontal(&[block("x"), tall.clone()], Align::Start),
            "x1\n 2\n 3"
        );
        assert_eq!(
            horizontal(&[block("x"), tall.clone()], Align::End),
            " 1\n 2\nx3"
        );
        assert_eq!(horizontal(&[block("x"), tall], Align::Middle), " 1\nx2\n 3");
    }

    /// Colour must not throw the alignment off. This is the whole reason `paste` will not do:
    /// escapes are zero cells wide and every byte-counting tool gets it wrong.
    #[test]
    fn escapes_do_not_count_toward_width() {
        let coloured = block("\x1b[31mab\x1b[0m\ncd");
        let out = horizontal(&[coloured, block("1\n2")], Align::Start);
        // Two visible cells from the first block, then the second's.
        for line in out.lines() {
            assert_eq!(printed_width(line), 3, "{line:?}");
        }
    }

    #[test]
    fn vertical_stacks_and_aligns() {
        assert_eq!(
            vertical(&[block("aaa"), block("b")], Align::Start),
            "aaa\nb"
        );
        assert_eq!(
            vertical(&[block("aaa"), block("b")], Align::End),
            "aaa\n  b"
        );
        assert_eq!(
            vertical(&[block("aaa"), block("b")], Align::Middle),
            "aaa\n b"
        );
    }

    #[test]
    fn an_alignment_that_is_not_one_is_refused() {
        assert_eq!(Align::parse("top"), Some(Align::Start));
        assert_eq!(Align::parse("center"), Some(Align::Middle));
        assert_eq!(Align::parse("right"), Some(Align::End));
        assert_eq!(Align::parse("sideways"), None);
    }

    #[test]
    fn joining_nothing_is_nothing() {
        assert_eq!(horizontal(&[], Align::Start), "");
        assert_eq!(vertical(&[], Align::Start), "");
    }
}
