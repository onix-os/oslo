//! Lining up a block of argc declarations.
//!
//! ```text
//! # @describe Deploy a thing
//! # @flag     -n --dry-run     say what would happen
//! # @option   -t --tries <N>   how many times
//! # @option      --verbose     noisier
//! # @arg      target!          where to
//! ```
//!
//! # Why a formatter that never touches comments touches these
//!
//! [`super`] promises it will not rewrite the text of a comment, and it keeps that promise for
//! prose. **An argc block is not prose.** It is a declaration of what a script takes, in the one
//! place a shell will let a declaration live, and it is read by a parser rather than by a person —
//! see `docs/features/argc.md`. Lining up a table is what a formatter is for; leaving this one
//! crooked because of where the language chose to put it would be honouring the letter of a rule
//! against its point.
//!
//! # Why padding is safe
//!
//! argc's `parse_tail` is `preceded(space1, rest.trim())` — every run of whitespace between two
//! tokens is already discarded, and every description is already trimmed. So no amount of padding
//! can change what argc parses out of a line, and nothing here has to know what any of the fields
//! *mean*. It splits on whitespace and joins with whitespace.
//!
//! # What it recognises
//!
//! A tag line is `#`, at **column zero**, then `@` and a tag word. Column zero because that is
//! argc's own rule: `parse_tag` starts at `many1(char('#'))` against the whole line, so an indented
//! `# @option` is a comment and not a declaration, and lining one up would be dressing something up
//! as a thing it is not.
//!
//! A *block* is a maximal run of them. A plain comment ends one, because `@describe` and `@cmd`
//! continue onto the comment lines under them — that text belongs to the tag above it, and moving
//! it into a column would be reformatting a sentence.
//!
//! # What it does with something it cannot read
//!
//! Leaves it alone. An unknown tag is laid out as text, and a description that begins with `<` or
//! `-` may be taken for part of the signature and pushed one column left. Both are cosmetic: the
//! tokens and their order never change, so the worst case is a line that is not improved.
//!
//! Not to be confused with [`crate::cli::argc`], which is the completion provider for the same
//! declarations.

/// Line up every argc block in a script.
pub(super) fn align(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < lines.len() {
        let Some(_) = tagged(lines[at]) else {
            out.push_str(lines[at]);
            out.push('\n');
            at += 1;
            continue;
        };
        let end = lines[at..]
            .iter()
            .position(|line| tagged(line).is_none())
            .map_or(lines.len(), |offset| at + offset);
        for line in block(&lines[at..end]) {
            out.push_str(&line);
            out.push('\n');
        }
        at = end;
    }
    // `lines()` drops the final newline and the loop puts one back after every line. Whether the
    // script ended in one is not this pass's business to change, so take back exactly the one that
    // was added.
    if !text.ends_with('\n') {
        out.pop();
    }
    out
}

/// One tag line, taken apart.
struct Tagged<'a> {
    /// The run of `#`, kept as written: `##` is a tag to argc too.
    hashes: &'a str,
    /// `@option`, with the `@`.
    tag: &'a str,
    /// Everything after the tag.
    rest: &'a str,
}

impl Tagged<'_> {
    /// What goes in the first column.
    fn head(&self) -> String {
        format!("{} {}", self.hashes, self.tag)
    }
}

/// A line argc would read as a declaration, or `None`.
fn tagged(line: &str) -> Option<Tagged<'_>> {
    let hashes = line.len() - line.trim_start_matches('#').len();
    if hashes == 0 {
        return None;
    }
    let after = line[hashes..].trim_start();
    if !after.starts_with('@') {
        return None;
    }
    let word = &after[1..];
    let end = word.find(|c: char| !is_tag_char(c)).unwrap_or(word.len());
    let tag = &after[..end + 1];
    // A bare `@` is a comment about something, not a tag.
    if tag.len() == 1 {
        return None;
    }
    Some(Tagged {
        hashes: &line[..hashes],
        tag,
        rest: after[tag.len()..].trim(),
    })
}

fn is_tag_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Whether a tag's rest is a signature and a description, or one run of text.
///
/// **Unknown means text**, which is the conservative answer: a tag this does not know has fields
/// nobody here can name, and guessing at them would move words about inside a line whose meaning is
/// somebody else's.
fn has_fields(tag: &str) -> bool {
    matches!(tag, "@flag" | "@option" | "@arg" | "@env" | "@meta")
}

/// One block, laid out.
fn block(lines: &[&str]) -> Vec<String> {
    let read: Vec<Tagged> = lines.iter().filter_map(|line| tagged(line)).collect();
    let split: Vec<Option<Fields>> = read
        .iter()
        .map(|line| has_fields(line.tag).then(|| fields(line.tag, line.rest)))
        .collect();

    let head = widest(read.iter().map(|line| line.head().chars().count()));
    // **Only lines that have a long spelling widen the flag column.** `@arg target!` keeps its name
    // in the same slot a short flag would use, and letting a long name set that column would push
    // every `--long` in the block out to meet it.
    let short = widest(
        split
            .iter()
            .flatten()
            .filter(|fields| !fields.long.is_empty())
            .map(|fields| fields.short.chars().count()),
    );
    // Where a description begins: past the widest signature there is, wherever its parts ended up.
    let signature = widest(split.iter().flatten().map(|fields| fields.width(short)));

    read.iter()
        .zip(&split)
        .map(|(line, split)| one(line, split.as_ref(), head, short, signature))
        .collect()
}

/// The widest of some widths, or zero if there are none.
fn widest(widths: impl Iterator<Item = usize>) -> usize {
    widths.max().unwrap_or(0)
}

/// A declaration's parts. Everything is a string here; nothing knows what any of it means.
struct Fields {
    /// `-n`, `+x`, the name of an `@arg` — or nothing.
    short: String,
    /// The long spellings and their notation. Empty for `@arg` and `@env`, whose name is not a
    /// flag and has nothing to sit beside.
    long: String,
    description: String,
}

impl Fields {
    /// How wide this signature is once the block's columns are applied to it.
    ///
    /// A name on its own is as wide as it is; a flag and its long spellings take the flag column,
    /// a space, and whatever the long part needs. Either way this is where the description starts
    /// measuring from.
    fn width(&self, short: usize) -> usize {
        if self.long.is_empty() {
            return self.short.chars().count();
        }
        match self.wants_a_flag_column(short) {
            true => self.short.chars().count().max(short) + 1 + self.long.chars().count(),
            false => self.long.chars().count(),
        }
    }

    /// Whether a slot has to be left for a short flag.
    ///
    /// **No, when nothing in the block has one.** A block of long-only options would otherwise
    /// carry a column of two blanks down its whole length, held open for a flag nobody wrote.
    fn wants_a_flag_column(&self, short: usize) -> bool {
        short > 0 || !self.short.is_empty()
    }
}

/// Split a declaration into what it declares and what it says about it.
///
/// The signature is the leading run of tokens that look like one: a flag spelling, a notation, a
/// bracketed default or list of choices — and, for `@arg`, `@env` and `@meta`, the first token
/// whatever it looks like, because that is the name.
fn fields(tag: &str, rest: &str) -> Fields {
    let mut words = rest.split_whitespace().peekable();
    let mut short = String::new();
    let mut long: Vec<&str> = Vec::new();

    match tag {
        // The name is the first word, and it is a name rather than a flag: it goes where `-n`
        // goes, and the flag column is left to the flags.
        "@arg" | "@env" | "@meta" => {
            if let Some(name) = words.next() {
                short = name.to_string();
            }
        }
        _ => {
            if words.peek().is_some_and(|word| is_short(word)) {
                short = words.next().unwrap_or_default().to_string();
            }
        }
    }
    while words.peek().is_some_and(|word| is_signature(word)) {
        long.push(words.next().unwrap_or_default());
    }

    Fields {
        short,
        long: long.join(" "),
        description: words.collect::<Vec<_>>().join(" "),
    }
}

/// `-n`, `+x` — one dash and one name, which is what argc calls a short flag.
fn is_short(word: &str) -> bool {
    let name = word.trim_start_matches(['-', '+']);
    word.len() - name.len() == 1 && !name.is_empty()
}

/// Still part of what is being declared rather than the start of the sentence about it.
fn is_signature(word: &str) -> bool {
    word.starts_with(['-', '+', '<', '['])
}

/// One line, in its columns.
fn one(
    line: &Tagged,
    split: Option<&Fields>,
    head: usize,
    short: usize,
    signature: usize,
) -> String {
    let mut out = line.head();
    let Some(fields) = split else {
        // `@describe`, `@cmd`, `@alias`: the text follows the tag and lines up with the signatures
        // rather than with the descriptions, because it *is* the whole of what the tag says.
        if !line.rest.is_empty() {
            fill(&mut out, head + 1);
            out.push_str(line.rest);
        }
        return out;
    };
    if fields.short.is_empty() && fields.long.is_empty() && fields.description.is_empty() {
        return out;
    }
    fill(&mut out, head + 1);

    let opened = out.chars().count();
    out.push_str(&fields.short);
    if !fields.long.is_empty() {
        if fields.wants_a_flag_column(short) {
            fill(&mut out, opened + short + 1);
        }
        out.push_str(&fields.long);
    }
    if !fields.description.is_empty() {
        fill(&mut out, opened + signature + DESCRIPTION_GAP);
        out.push_str(&fields.description);
    }
    out
}

/// How far the description column sits from the longest signature in the block.
///
/// **Three rather than one**, and it is the only gap here that is not one. The description column
/// is the one the eye runs down; set a single space from the widest signature above it, it reads as
/// attached to *that line* rather than as a column of its own, and the whole point of lining a table
/// up is lost on the one row that made it that wide.
const DESCRIPTION_GAP: usize = 3;

/// Pad so the next thing starts at `column`, and always leave at least one space.
fn fill(out: &mut String, column: usize) {
    let width = out.chars().count();
    for _ in width..column.max(width + 1) {
        out.push(' ');
    }
}

#[cfg(test)]
#[path = "argc/tests.rs"]
mod tests;
