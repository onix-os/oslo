//! `detect-columns` — somebody else's aligned output, as rows.
//!
//! ```text
//! docker ps | detect-columns | where 'STATUS:match("^Up")' | cols NAMES IMAGE
//! kubectl get pods | detect-columns | cols NAME STATUS
//! ps aux | detect-columns | sort-by -r %MEM | first 5
//! ```
//!
//! # Why this is the verb that matters most
//!
//! `parse` needs a pattern and `from json` needs the program to speak JSON. Most programs do
//! neither: they print a header and some columns lined up with spaces, and that is the *entire*
//! interface a shell has to almost everything installed. `detect-columns` reads that, with nothing
//! to write and nothing for the other program to agree to — which is the same argument `bridge`
//! opens with, taken one step further.
//!
//! # How the columns are found
//!
//! Three rules, and each exists because the ones before it are not enough.
//!
//! **1. Every line agrees.** A position separates columns when it is whitespace on the header *and*
//! every row. This is the main rule, and it finds boundaries the header alone cannot:
//!
//! ```text
//! USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND
//! root           1  0.0  0.0  27684 12156 ?        Ss   Aug23   3:01 /usr/lib/systemd/systemd …
//! ```
//!
//! `PID %CPU %MEM` are one space apart in the header because they are right-aligned into narrow
//! columns, so splitting the header on two-or-more spaces reads them as one column and shifts every
//! field after it. The *data* is aligned even where the header is not.
//!
//! **2. The header left a wide gap.** Rule 1 is defeated by a single wide value: one process with an
//! eight-digit `VSZ` closes the gap between `VSZ` and `RSS` for the whole table, because the rule
//! asks every row. So a boundary is also taken wherever the *header alone* has two or more spaces,
//! which no row can close. Two spaces rather than one, because one space is inside a name —
//! `CONTAINER ID` — and two is between columns.
//!
//! **3. A cut never lands inside a word.** Rule 2 can put a boundary in the middle of a value that
//! outgrew its column — a six-digit `PID` under a three-wide heading — so each cut snaps back to the
//! start of the word it landed in. Slicing `123456` into `123` and `456` is a wrong answer; giving
//! the whole value to one column is a ragged one.
//!
//! The last column keeps its spaces — `COMMAND`, `NAMES`, `STATUS` all contain them — because
//! nothing inside it is whitespace on every row and the header has no gap there.
//!
//! If nothing lines up at all, the text is separated but not padded, and there is nothing to align
//! to: it falls back to splitting on whitespace with the tail kept whole.
//!
//! # What it cannot do
//!
//! * **A column that is empty on every row is invisible.** It has no non-whitespace position
//!   anywhere, so it is part of a gap.
//! * **Two columns the header packs one space apart, whose values also touch on some row, stay
//!   merged.** `ps aux` does this with `RSS TTY` on a busy machine: neither rule can see a boundary
//!   that is not there on any line. `ps` is a poor example to reach for anyway — oslo has a native
//!   `ps` producer that reads `/proc` and never guesses.
//!
//! Both want `parse` with a pattern, which is why that verb is not going anywhere.

use crate::data::{Record, Val};

/// How the text was said to be laid out.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The first line is data, not names: columns are called `column0`, `column1`, …
    pub no_headers: bool,
    /// Lines to drop before the header — a banner, a blank, a `Total:` line.
    pub skip: usize,
}

/// Rows from aligned text.
pub fn detect(input: &str, layout: Layout) -> Vec<Record> {
    let lines: Vec<Vec<char>> = input
        .lines()
        .skip(layout.skip)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.chars().collect())
        .collect();
    let Some((header, rest)) = lines.split_first() else {
        return Vec::new();
    };

    let mut starts = column_starts(&lines, header);
    // **A column the header has nothing in is not a column.** A line longer than the rest makes the
    // positions past every other line's end look like a gap — they are "whitespace" only because
    // those lines ended — and the next character of the long line then opens a column nothing has a
    // name for. `ps aux`, whose last field is the longest line by far, grew a phantom column that
    // way and every real one merged into the first.
    if !layout.no_headers {
        let named = cut(header, &starts);
        starts = starts
            .into_iter()
            .zip(named)
            .filter(|(_, name)| !name.is_empty())
            .map(|(at, _)| at)
            .collect();
    }
    if starts.is_empty() {
        return Vec::new();
    }
    // **Nothing lined up.** Output that is single-space separated and *not* padded — some `--porcelain`
    // formats, some hand-written scripts — has no position that is whitespace on every line, so the
    // alignment rule finds one column. Splitting on whitespace is the only thing left, and it is
    // right often enough to beat answering with the whole line.
    if starts.len() < 2 && lines.iter().any(|line| tokens(line).len() > 1) {
        return unaligned(&lines, layout);
    }

    let names: Vec<String> = match layout.no_headers {
        false => cut(header, &starts),
        true => (0..starts.len()).map(|i| format!("column{i}")).collect(),
    };
    // With no header the first line is data, so it is read again as one.
    let data: &[Vec<char>] = match layout.no_headers {
        true => &lines,
        false => rest,
    };
    data.iter()
        .map(|line| {
            let mut record = Record::new();
            for (name, cell) in names.iter().zip(cut(line, &starts)) {
                if name.is_empty() {
                    continue;
                }
                // An empty cell is absent rather than blank, so `compact` and `default` can tell a
                // gap from a value that is genuinely the empty string.
                if !cell.is_empty() {
                    record.set(name, scalar(&cell));
                }
            }
            record
        })
        .collect()
}

/// The whitespace-separated words of a line.
fn tokens(line: &[char]) -> Vec<String> {
    line.iter()
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Rows from text that is separated but not aligned.
///
/// The tail joins into the last column, because the thing that runs on is nearly always a path, a
/// message or a command line — splitting it into anonymous extra columns would be worse than
/// keeping it whole.
fn unaligned(lines: &[Vec<char>], layout: Layout) -> Vec<Record> {
    let Some((header, rest)) = lines.split_first() else {
        return Vec::new();
    };
    let heading = tokens(header);
    let names: Vec<String> = match layout.no_headers {
        false => heading,
        true => (0..heading.len()).map(|i| format!("column{i}")).collect(),
    };
    if names.is_empty() {
        return Vec::new();
    }
    let data: &[Vec<char>] = match layout.no_headers {
        true => lines,
        false => rest,
    };
    data.iter()
        .map(|line| {
            let mut cells = tokens(line);
            if cells.len() > names.len() {
                let tail = cells.split_off(names.len() - 1).join(" ");
                cells.push(tail);
            }
            let mut record = Record::new();
            for (name, cell) in names.iter().zip(cells) {
                record.set(name, scalar(&cell));
            }
            record
        })
        .collect()
}

/// Where each column begins.
///
/// **Two signals, unioned, because neither is enough alone.**
///
/// *Every line agrees* finds a boundary wherever one position is whitespace on the header and every
/// row. It is what separates `PID` from `%CPU` in `ps aux`, whose header writes them one space
/// apart. But a single wide value anywhere in the stream closes a gap for good — one process with
/// an eight-digit `VSZ` merges `VSZ` and `RSS` for the whole table.
///
/// *The header has a wide gap* finds a boundary wherever the header alone has two or more spaces.
/// It survives a wide value, because it never looks at the rows. But it misses every column the
/// header packs one space apart, and it splits a header name that contains a space — `CONTAINER ID`
/// — if the gap rule is loosened to one.
///
/// Each covers the other's blind spot, so a boundary either of them finds is a boundary.
fn column_starts(lines: &[Vec<char>], header: &[char]) -> Vec<usize> {
    let width = lines.iter().map(Vec::len).max().unwrap_or(0);
    // A line shorter than the widest counts as padded with spaces, so a ragged row does not close a
    // gap the rest of the table has.
    let agreed = |at: usize| {
        lines
            .iter()
            .all(|line| line.get(at).is_none_or(|c| c.is_whitespace()))
    };
    let blank = |line: &[char], at: usize| line.get(at).is_none_or(|c| c.is_whitespace());
    // Two or more, because one space is inside a name — `CONTAINER ID` — and two is between columns.
    let wide_in_header =
        |at: usize| at > 0 && blank(header, at - 1) && blank(header, at.wrapping_sub(2));

    let mut starts = Vec::new();
    let mut in_gap = true;
    for at in 0..width {
        let boundary = agreed(at);
        if boundary {
            in_gap = true;
            continue;
        }
        // A column opens after any gap, and also wherever the header left two spaces — even if some
        // row has since filled them.
        if in_gap || (wide_in_header(at) && !blank(header, at)) {
            starts.push(at);
        }
        in_gap = false;
    }
    starts.dedup();
    starts
}

/// One line cut at the column starts, each piece trimmed.
///
/// A column runs to the *start of the next one*, so the padding after a value belongs to it and a
/// value that overflows its header is not truncated.
fn cut(line: &[char], starts: &[usize]) -> Vec<String> {
    // **A cut never lands inside a word.** A boundary the header asked for can fall in the middle of
    // a value that outgrew its column — a six-digit `PID` under a three-wide `PID` heading — and
    // slicing it there turns `123456` into `123` and `456`, which is a wrong answer rather than a
    // ragged one. Snapping to the start of the word it landed in gives the whole value to the
    // column it belongs to.
    let mut snapped: Vec<usize> = Vec::with_capacity(starts.len());
    for &at in starts {
        let mut at = at.min(line.len());
        while at > 0
            && at < line.len()
            && !line[at].is_whitespace()
            && !line[at - 1].is_whitespace()
        {
            at -= 1;
        }
        // Never behind the boundary before it, or two columns would claim the same characters.
        if let Some(&previous) = snapped.last() {
            at = at.max(previous);
        }
        snapped.push(at);
    }
    snapped
        .iter()
        .enumerate()
        .map(|(i, &from)| {
            let to = snapped.get(i + 1).copied().unwrap_or(line.len());
            let to = to.min(line.len()).max(from);
            line[from..to].iter().collect::<String>().trim().to_string()
        })
        .collect()
}

/// A cell as the most specific kind it plainly is — the same rule `parse` follows, so a column of
/// numbers compares as numbers wherever it came from.
fn scalar(text: &str) -> Val {
    let trimmed = text.trim();
    if let Ok(i) = trimmed.parse::<i64>() {
        return Val::Int(i);
    }
    if let Ok(f) = trimmed.parse::<f64>()
        && trimmed.contains('.')
    {
        return Val::Float(f);
    }
    Val::Str(trimmed.to_string())
}

#[cfg(test)]
#[path = "detect/tests.rs"]
mod tests;
