//! Safe editor text and the coordinate conversions used to draw it.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Span {
    raw_start: usize,
    raw_end: usize,
    rendered_start: usize,
    rendered_end: usize,
    text: String,
}

/// A terminal-safe view of a raw command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayMap {
    plain: String,
    spans: Vec<Span>,
    raw_len: usize,
}

impl DisplayMap {
    pub fn new(raw: &str) -> Self {
        let mut plain = String::with_capacity(raw.len());
        let mut spans = Vec::new();
        let mut raw_at = 0usize;
        let mut rendered_at = 0usize;

        for grapheme in raw.graphemes(true) {
            let raw_len = grapheme.chars().count();
            let text = safe_grapheme(grapheme);
            let rendered_len = text.chars().count();
            spans.push(Span {
                raw_start: raw_at,
                raw_end: raw_at + raw_len,
                rendered_start: rendered_at,
                rendered_end: rendered_at + rendered_len,
                text: text.clone(),
            });
            plain.push_str(&text);
            raw_at += raw_len;
            rendered_at += rendered_len;
        }

        Self {
            plain,
            spans,
            raw_len: raw_at,
        }
    }

    pub fn plain(&self) -> &str {
        &self.plain
    }

    pub fn rendered_cursor(&self, raw_cursor: usize) -> usize {
        if raw_cursor >= self.raw_len {
            return self.plain.chars().count();
        }
        let cursor = clamp_char_boundary_from_spans(&self.spans, raw_cursor.min(self.raw_len));
        self.spans
            .iter()
            .find(|span| span.raw_start == cursor)
            .map(|span| span.rendered_start)
            .unwrap_or_else(|| self.plain.chars().count())
    }

    pub fn raw_cursor(&self, rendered_cursor: usize) -> usize {
        let cursor = rendered_cursor.min(self.plain.chars().count());
        for span in &self.spans {
            if cursor <= span.rendered_start {
                return span.raw_start;
            }
            if cursor < span.rendered_end {
                return span.raw_start;
            }
        }
        self.raw_len
    }

    pub fn cursor_for_cell(
        &self,
        prompt_cells: usize,
        cols: usize,
        target_row: usize,
        target_col: usize,
    ) -> usize {
        let cols = cols.max(1);
        let target = target_row
            .saturating_mul(cols)
            .saturating_add(target_col.min(cols - 1));
        let mut cell = prompt_cells;
        if target <= cell {
            return 0;
        }

        for span in &self.spans {
            if span.text == "\n" {
                cell = (cell / cols + 1).saturating_mul(cols);
                if target < cell {
                    return span.raw_end;
                }
                continue;
            }
            let width = UnicodeWidthStr::width(span.text.as_str());
            let column = cell % cols;
            if width > 1 && column > 0 && column.saturating_add(width) > cols {
                let wrapped = cell.saturating_add(cols - column);
                if target < wrapped {
                    return span.raw_start;
                }
                cell = wrapped;
            }
            if target < cell.saturating_add(width) {
                return if target.saturating_sub(cell) * 2 < width {
                    span.raw_start
                } else {
                    span.raw_end
                };
            }
            cell = cell.saturating_add(width);
        }
        self.raw_len
    }
}

/// Advance an absolute terminal cell position through safe plain text.
pub fn advance_cells(mut cell: usize, text: &str, cols: usize) -> usize {
    let cols = cols.max(1);
    for grapheme in text.graphemes(true) {
        if grapheme == "\n" {
            cell = (cell / cols + 1).saturating_mul(cols);
            continue;
        }
        let width = UnicodeWidthStr::width(grapheme);
        let column = cell % cols;
        if width > 1 && column > 0 && column.saturating_add(width) > cols {
            cell = cell.saturating_add(cols - column);
        }
        cell = cell.saturating_add(width);
    }
    cell
}

fn safe_grapheme(grapheme: &str) -> String {
    let mut out = String::with_capacity(grapheme.len());
    for character in grapheme.chars() {
        match character {
            '\n' => out.push('\n'),
            '\x00'..='\x1f' => {
                out.push('^');
                out.push(char::from_u32(character as u32 + 0x40).unwrap_or('?'));
            }
            '\x7f' => out.push_str("^?"),
            c if c.is_control() => out.push_str(&format!("U+{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn raw_boundaries(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    let mut at = 0usize;
    for grapheme in text.graphemes(true) {
        at += grapheme.chars().count();
        out.push(at);
    }
    out
}

fn clamp_char_boundary_from_spans(spans: &[Span], at: usize) -> usize {
    spans
        .iter()
        .map(|span| span.raw_start)
        .take_while(|boundary| *boundary <= at)
        .last()
        .unwrap_or(0)
}

/// Clamp a character offset to the preceding extended-grapheme boundary.
pub fn clamp_char_boundary(text: &str, at: usize) -> usize {
    raw_boundaries(text)
        .into_iter()
        .take_while(|boundary| *boundary <= at)
        .last()
        .unwrap_or(0)
}

/// The boundary before `at`, or zero.
pub fn previous_boundary(text: &str, at: usize) -> usize {
    raw_boundaries(text)
        .into_iter()
        .take_while(|boundary| *boundary < at)
        .last()
        .unwrap_or(0)
}

/// The first boundary after `at`, or the end.
pub fn next_boundary(text: &str, at: usize) -> usize {
    let end = text.chars().count();
    raw_boundaries(text)
        .into_iter()
        .find(|boundary| *boundary > at)
        .unwrap_or(end)
}

/// Move by `count` grapheme clusters from a legal character boundary.
pub fn move_boundaries(text: &str, at: usize, count: isize) -> usize {
    let boundaries = raw_boundaries(text);
    let clamped = clamp_char_boundary(text, at);
    let index = boundaries
        .binary_search(&clamped)
        .unwrap_or_else(|index| index.saturating_sub(1));
    let target = if count < 0 {
        index.saturating_sub(count.unsigned_abs())
    } else {
        index
            .saturating_add(count as usize)
            .min(boundaries.len() - 1)
    };
    boundaries[target]
}

/// Convert a legal character cursor to a UTF-8 byte cursor.
pub fn char_to_byte(text: &str, at: usize) -> usize {
    text.chars()
        .take(clamp_char_boundary(text, at))
        .map(char::len_utf8)
        .sum()
}

/// Convert a byte cursor to the preceding legal character cursor.
pub fn byte_to_char(text: &str, at: usize) -> usize {
    let mut byte = at.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    let chars = text
        .char_indices()
        .take_while(|(start, _)| *start < byte)
        .count();
    clamp_char_boundary(text, chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_are_visible_and_raw_coordinates_round_trip() {
        let map = DisplayMap::new("a\x1b\t\r\0\x7f\u{85}z");
        assert_eq!(map.plain(), "a^[^I^M^@^?U+0085z");
        assert!(!map.plain().chars().any(|c| c.is_control()));
        for raw in 0..=8 {
            assert_eq!(map.raw_cursor(map.rendered_cursor(raw)), raw);
        }
    }

    #[test]
    fn line_feeds_remain_logical_lines() {
        let map = DisplayMap::new("one\ntwo");
        assert_eq!(map.plain(), "one\ntwo");
        assert_eq!(map.cursor_for_cell(2, 10, 1, 2), 6);
    }

    #[test]
    fn grapheme_boundaries_cover_combining_emoji_and_flags() {
        for grapheme in ["e\u{301}", "👍🏽", "👨‍👩‍👧‍👦", "🇳🇱", "1️⃣", "क्‍ष"]
        {
            let text = format!("a{grapheme}z");
            let end = text.chars().count();
            assert_eq!(next_boundary(&text, 1), end - 1, "{grapheme:?}");
            assert_eq!(previous_boundary(&text, end - 1), 1, "{grapheme:?}");
            for inside in 2..end - 1 {
                assert_eq!(clamp_char_boundary(&text, inside), 1, "{grapheme:?}");
            }
        }
    }

    #[test]
    fn byte_offsets_inside_a_cluster_clamp_backward() {
        let text = "ae\u{301}日";
        let cluster_start = text.find('e').unwrap();
        let cluster_end = text.find('日').unwrap();
        for byte in cluster_start + 1..cluster_end {
            assert_eq!(byte_to_char(text, byte), 1);
        }
        assert_eq!(char_to_byte(text, 3), cluster_end);
    }

    #[test]
    fn control_cells_map_only_to_raw_boundaries() {
        let map = DisplayMap::new("a\x1bz");
        assert_eq!(map.cursor_for_cell(0, 80, 0, 1), 1);
        assert_eq!(map.cursor_for_cell(0, 80, 0, 2), 2);
        assert_eq!(map.cursor_for_cell(0, 80, 0, 3), 2);
        assert_eq!(map.cursor_for_cell(0, 80, 0, 4), 3);
    }

    #[test]
    fn wide_graphemes_wrap_before_a_short_final_cell() {
        assert_eq!(advance_cells(1, "日", 2), 4);
        let map = DisplayMap::new("a日z");
        assert_eq!(map.cursor_for_cell(0, 2, 0, 1), 1);
        assert_eq!(map.cursor_for_cell(0, 2, 1, 0), 1);
        assert_eq!(map.cursor_for_cell(0, 2, 1, 1), 2);
    }
}
