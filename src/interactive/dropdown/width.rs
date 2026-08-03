//! Measuring things in terminal cells: string widths, wrap counts, and the terminal's own size.
//!
//! Everything the dropdown draws is clamped against numbers produced here, so a miscount by one
//! cell is a wrapped row, and a wrapped row is a cursor-up that walks too few lines and paints
//! over the prompt (R9.4).

use nix::libc;

/// Column count assumed when the terminal will not say how wide it is.
pub const FALLBACK_COLS: usize = 80;

/// How many terminal cells `s` occupies, ignoring SGR escape sequences.
///
/// This is an approximation of Unicode's East Asian Width plus emoji presentation — enough for
/// the icons and file names a completion list actually contains, and deliberately dependency
/// free. The two rules that decide the icon column are handled exactly: a variation selector
/// U+FE0F forces the character before it to emoji (two-cell) presentation, and zero-width
/// joiners and combining marks add nothing.
pub fn display_width(s: &str) -> usize {
    let mut width = 0usize;
    let mut in_esc = false;
    let mut prev: Option<char> = None;

    for c in s.chars() {
        if in_esc {
            // CSI sequences end at their final byte; SGR (`m`) is all this code emits.
            if c.is_ascii_alphabetic() {
                in_esc = false;
            }
            continue;
        }
        match c {
            '\x1b' => in_esc = true,
            // Emoji presentation selector: widens the character it follows to two cells.
            '\u{fe0f}' => {
                if let Some(p) = prev
                    && char_width(p) == 1
                {
                    width += 1;
                }
            }
            _ => width += char_width(c),
        }
        if !in_esc {
            prev = Some(c);
        }
    }
    width
}

fn char_width(c: char) -> usize {
    let cp = c as u32;
    match cp {
        // Zero width: joiners, text/emoji selectors, combining marks.
        0x200b..=0x200f | 0xfe00..=0xfe0f | 0x0300..=0x036f | 0x1ab0..=0x1aff => 0,
        // Wide: CJK, Hangul, fullwidth forms.
        0x1100..=0x115f
        | 0x2e80..=0x303e
        | 0x3041..=0x33ff
        | 0x3400..=0x4dbf
        | 0x4e00..=0x9fff
        | 0xa000..=0xa4cf
        | 0xac00..=0xd7a3
        | 0xf900..=0xfaff
        | 0xfe30..=0xfe6f
        | 0xff00..=0xff60
        | 0xffe0..=0xffe6 => 2,
        // Wide: the emoji blocks the icon set draws from.
        0x1f300..=0x1faff | 0x1f000..=0x1f0ff | 0x26a1 | 0x2b1b..=0x2b1c | 0x2b50 => 2,
        _ if cp < 0x20 => 0,
        _ => 1,
    }
}

/// Kept for callers that measure a prompt before indenting the dropdown under it; cells, not
/// bytes, is what an indent needs.
pub fn visible_len(s: &str) -> usize {
    display_width(s)
}

/// Split `s` into indivisible display cells: a base character together with the zero-width
/// characters that modify it, paired with the width of the pair.
///
/// Truncating per `char` instead would measure `⚙` and its variation selector separately — one
/// cell instead of two — and hand back a "clamped" string that is a cell too wide, which is the
/// wrap this module exists to prevent. Assumes plain text; labels and descriptions carry no
/// escapes of their own.
fn cells(s: &str) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for c in s.chars() {
        let w = char_width(c);
        if w == 0
            && let Some(last) = out.last_mut()
        {
            // U+FE0F promotes its base to emoji (two-cell) presentation.
            if c == '\u{fe0f}' && last.1 == 1 {
                last.1 = 2;
            }
            last.0.push(c);
            continue;
        }
        out.push((c.to_string(), w));
    }
    out
}

/// Cut `s` down to `max` cells, marking the cut with `…` so the reader knows something was
/// dropped.
pub fn truncate_to_width(s: &str, max: usize) -> String {
    if display_width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    // One cell is reserved for the ellipsis.
    let budget = max - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for (text, w) in cells(s) {
        if used + w > budget {
            break;
        }
        out.push_str(&text);
        used += w;
    }
    out.push('…');
    out
}

/// `s` with every escape sequence removed — what the terminal actually shows.
///
/// The counterpart to [`display_width`], which measures the same thing without building it. A
/// caller writing to a pipe, a log or a test wants the text; a caller writing to a terminal wants
/// the escapes left alone.
pub fn without_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        // CSI and OSC both end at a byte this can recognise: CSI at its alphabetic final byte, OSC
        // at BEL or ST. Anything else is a two-character sequence whose second character is eaten.
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Terminal height in rows, by the same route as [`terminal_cols`].
///
/// `$LINES` rather than `$COLUMNS` as the fallback, and 24 as the last resort — the size of the
/// terminal every fallback in this file is descended from.
pub fn terminal_rows() -> usize {
    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO, libc::STDIN_FILENO] {
        if let Some(rows) = winsize_rows(fd) {
            return rows;
        }
    }
    std::env::var("LINES")
        .ok()
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(24)
}

fn winsize_rows(fd: i32) -> Option<usize> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: as `winsize_cols` — a live, correctly typed `struct winsize`, written only by the
    // ioctl, which fails rather than writing when the fd is not a terminal.
    let rc = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) };
    (rc == 0 && ws.ws_row > 0).then_some(ws.ws_row as usize)
}

/// Right-pad `s` with spaces to `target` cells; a no-op when it is already at least that wide.
pub fn pad_to_width(s: &str, target: usize) -> String {
    let w = display_width(s);
    let mut out = s.to_string();
    if w < target {
        out.push_str(&" ".repeat(target - w));
    }
    out
}

/// How many screen rows a logical row of `width` cells occupies in a `cols`-wide terminal.
///
/// The `-1` is not an off-by-one: a row that fills the last column leaves the cursor in that
/// column with a *pending* wrap, so it has not yet consumed a second row. Counting it as two
/// would walk the cursor up one row too far and eat the line above the prompt.
pub fn physical_rows(width: usize, cols: usize) -> usize {
    if cols == 0 || width <= 1 {
        return 1;
    }
    1 + (width - 1) / cols
}

/// Terminal width in columns, from `TIOCGWINSZ` on whichever standard stream is still a terminal,
/// then `$COLUMNS`, then [`FALLBACK_COLS`]. Never returns 0: a zero width would make every layout
/// computation collapse.
pub fn terminal_cols() -> usize {
    // stdout first: that is where the dropdown is drawn. But a shell whose stdout is redirected
    // still draws its UI on the terminal it kept, so stderr and stdin are asked in turn.
    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO, libc::STDIN_FILENO] {
        if let Some(cols) = winsize_cols(fd) {
            return cols;
        }
    }
    // A terminal that answers no ioctl may still have had its size exported; bash keeps COLUMNS
    // up to date for exactly this.
    if let Ok(v) = std::env::var("COLUMNS")
        && let Ok(n) = v.trim().parse::<usize>()
        && n > 0
    {
        return n;
    }
    FALLBACK_COLS
}

fn winsize_cols(fd: i32) -> Option<usize> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: `ws` is a live, correctly typed `struct winsize` for the duration of the call and
    // TIOCGWINSZ only writes into it. A non-terminal fd fails with ENOTTY rather than writing.
    let rc = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 {
        Some(ws.ws_col as usize)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_width_counts_cells_not_bytes() {
        assert_eq!(display_width("cargo"), 5);
        assert_eq!(display_width("\x1b[1;36mcargo\x1b[0m"), 5);
        // Emoji icons are two cells plus their trailing space.
        assert_eq!(display_width("📁 "), 3);
        assert_eq!(display_width("🚩 "), 3);
        // A variation selector widens its base rather than adding a cell of its own.
        assert_eq!(display_width("⚙️ "), 3);
        assert_eq!(display_width("🏷️ "), 3);
        // Box-drawing characters are one cell each, or the borders would not line up.
        assert_eq!(display_width("╭─╮"), 3);
        assert_eq!(display_width("│ ▶ │"), 5);
    }

    #[test]
    fn cells_agree_with_display_width() {
        // Truncation walks `cells` while clamping compares against `display_width`; if the two
        // disagree the "clamped" string is still too wide.
        for s in ["cargo", "📁 dir/", "⚙️ x", "🏷️ sub", "ünïcödé"] {
            let summed: usize = cells(s).iter().map(|(_, w)| w).sum();
            assert_eq!(summed, display_width(s), "disagreement on {s:?}");
        }
    }

    #[test]
    fn physical_rows_counts_wraps_with_pending_wrap_semantics() {
        assert_eq!(physical_rows(0, 80), 1);
        assert_eq!(physical_rows(40, 80), 1);
        // A row that exactly fills the width leaves a *pending* wrap: still one row.
        assert_eq!(physical_rows(80, 80), 1);
        assert_eq!(physical_rows(81, 80), 2);
        // The measured R9.4 case: 297-cell rows on an 80-column terminal.
        assert_eq!(physical_rows(297, 80), 4);
        assert_eq!(physical_rows(160, 80), 2);
        assert_eq!(physical_rows(161, 80), 3);
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate_to_width("cargo", 10), "cargo");
        assert_eq!(truncate_to_width("cargo", 5), "cargo");
        assert_eq!(truncate_to_width("cargo", 4), "car…");
        assert_eq!(truncate_to_width("cargo", 1), "…");
        assert_eq!(truncate_to_width("cargo", 0), "");
        // Never leaves half a wide character behind, and never exceeds the budget.
        assert_eq!(display_width(&truncate_to_width("📁📁📁", 5)), 5);
        assert_eq!(display_width(&truncate_to_width("⚙️ cargo", 4)), 4);
    }

    #[test]
    fn pad_reaches_the_target_but_never_shrinks() {
        assert_eq!(pad_to_width("cd", 5), "cd   ");
        assert_eq!(display_width(&pad_to_width("⚙️ cd", 8)), 8);
        assert_eq!(pad_to_width("cargo", 2), "cargo");
    }

    #[test]
    fn terminal_cols_is_never_zero() {
        // Under `cargo test` there is no terminal, so this exercises the fallback path.
        assert!(terminal_cols() > 0);
    }
}
