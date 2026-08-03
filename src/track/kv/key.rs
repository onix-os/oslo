//! Composite keys, and the byte order that has to match the logical one.
//!
//! oslo's keys are tuples — `(dir_id, mode, argv)`, `(base, dir_id)`, `(mode, argv, dir_id)` — and
//! every range scan in the store is a *prefix* of one of them. A key-value engine sorts by bytes
//! and knows nothing about fields, so the encoding is the whole correctness argument: if the
//! encoded bytes do not sort in the same order as the fields, a prefix scan silently returns the
//! wrong rows and the symptom is "the suggestions are a bit off", which nobody debugs to here.
//!
//! # The separator, and why it is `0x00` and not `0xFF`
//!
//! A variable-length field needs a mark saying where it ends. Whatever that mark is, it is
//! compared against whatever byte a *longer* field has in the same position — so for the shorter
//! field to sort first, as it logically must, the mark has to be **lower than every byte a field
//! can contain**.
//!
//! `0xFF` fails that outright, and fails it quietly because it looks safe: `0xFF` and `0xFE` never
//! occur in valid UTF-8, so nobody has to escape anything. But `("cargo", …)` encodes as
//! `cargo FF …` and `("cargot", …)` as `cargot …`, and at the sixth byte that is `0xFF` against
//! `t` — so `cargo` sorts *after* `cargot`, and every prefix scan whose range ends at a field
//! boundary loses rows. Ordering, not escaping, is what decides the separator.
//!
//! `0x00` is the only choice that puts a prefix before its extensions, and it costs an escape,
//! because `0x00` is a byte a field may legitimately contain. Two bytes, so the escape can never
//! be confused with the terminator:
//!
//! * a literal `0x00` inside a field becomes `0x00 0x01`
//! * the end of a field is `0x00 0x00`
//!
//! `0x00 0x00` < `0x00 0x01` keeps "the shorter field sorts first" true, and the pair makes the
//! encoding self-delimiting: after a `0x00` the next byte is `0x00` (end) or `0x01` (a NUL) and
//! nothing else. That is what lets a variable field be followed by another field without a prefix
//! scan on the earlier one reaching into the later one — a one-byte terminator does not, and the
//! secondary index `(mode, argv, dir_id)` is exactly the shape where that goes wrong.
//!
//! # Integers
//!
//! Big-endian and fixed width, which is the same rule for the same reason: the most significant
//! byte has to be compared first, and eight bytes of known length need no terminator and must not
//! be escaped — escaping would preserve the order but destroy the fixed width the decoder reads.
//! Little-endian would sort `256` before `2`.
//!
//! # These are values too
//!
//! A row of counters is a tuple as much as a key is, so [`Key`] and [`Fields`] encode `run`'s
//! `runs`/`fails`/`last_at`/`total_ms`/`max_ms` just as well. The ordering guarantee is wasted
//! there and the self-describing framing is not, which is what stops a second encoder appearing.

use std::borrow::Cow;

/// The end of a variable-length field.
const END: [u8; 2] = [0x00, 0x00];

/// What a `0x00` inside a field is written as.
const ESCAPED_NUL: [u8; 2] = [0x00, 0x01];

/// A key under construction.
///
/// Fields are appended in order and the result is one `Vec<u8>`. Every key in one bucket must be
/// built from the same sequence of calls; two shapes in one bucket sort against each other by
/// accident rather than by design.
#[derive(Debug, Clone, Default)]
pub struct Key(Vec<u8>);

impl Key {
    /// An empty key.
    pub fn new() -> Key {
        Key(Vec::new())
    }

    /// An empty key with room for `bytes`, for the hot paths that know their size.
    pub fn with_capacity(bytes: usize) -> Key {
        Key(Vec::with_capacity(bytes))
    }

    /// An unsigned integer: eight bytes, big-endian, no terminator.
    pub fn int(mut self, value: u64) -> Key {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    /// A signed integer, ordered as a number rather than as a two's-complement bit pattern.
    ///
    /// The top bit is flipped, which maps `i64::MIN..=i64::MAX` onto `0..=u64::MAX` in order —
    /// without it every negative would sort above every positive, because a negative's sign bit is
    /// set. `dir.missing_since` and a status are the fields that need it.
    pub fn signed(self, value: i64) -> Key {
        self.int((value as u64) ^ (1 << 63))
    }

    /// A whole text field, escaped and terminated.
    pub fn text(self, value: &str) -> Key {
        self.blob(value.as_bytes())
    }

    /// A whole byte field, escaped and terminated.
    pub fn blob(mut self, value: &[u8]) -> Key {
        escape(&mut self.0, value);
        self.0.extend_from_slice(&END);
        self
    }

    /// The beginning of a text field, escaped but **not** terminated.
    ///
    /// This is what a prefix scan is built from: `Key::new().int(dir).text(mode).text_prefix(typed)`
    /// names every key whose `argv` starts with `typed`. Nothing may follow it — an unterminated
    /// field has no end for the next one to begin after.
    pub fn text_prefix(self, value: &str) -> Key {
        self.blob_prefix(value.as_bytes())
    }

    /// The beginning of a byte field, escaped but not terminated. See [`Key::text_prefix`].
    pub fn blob_prefix(mut self, value: &[u8]) -> Key {
        escape(&mut self.0, value);
        self
    }

    /// The encoded key.
    pub fn done(self) -> Vec<u8> {
        self.0
    }
}

/// Append `value` with every `0x00` doubled into `0x00 0x01`.
fn escape(out: &mut Vec<u8>, value: &[u8]) {
    match value.iter().position(|byte| *byte == 0) {
        // The overwhelmingly common case: no NUL, so the field is copied straight in.
        None => out.extend_from_slice(value),
        Some(first) => {
            out.extend_from_slice(&value[..first]);
            for byte in &value[first..] {
                if *byte == 0 {
                    out.extend_from_slice(&ESCAPED_NUL);
                } else {
                    out.push(*byte);
                }
            }
        }
    }
}

/// A key being taken apart, in the order it was built.
///
/// Every read borrows from the key, so a field with no `0x00` in it — which is nearly all of them —
/// costs no allocation at all.
#[derive(Debug, Clone)]
pub struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    /// Read `key` back, field by field.
    pub fn of(key: &'a [u8]) -> Fields<'a> {
        Fields(key)
    }

    /// The next unsigned integer, or `None` if the key is too short.
    pub fn int(&mut self) -> Option<u64> {
        let (head, rest) = self.0.split_at_checked(8)?;
        self.0 = rest;
        Some(u64::from_be_bytes(head.try_into().ok()?))
    }

    /// The next signed integer, undoing the flip [`Key::signed`] applied.
    pub fn signed(&mut self) -> Option<i64> {
        Some((self.int()? ^ (1 << 63)) as i64)
    }

    /// The next text field, or `None` if it is unterminated or not UTF-8.
    pub fn text(&mut self) -> Option<Cow<'a, str>> {
        match self.blob()? {
            Cow::Borrowed(bytes) => std::str::from_utf8(bytes).ok().map(Cow::Borrowed),
            Cow::Owned(bytes) => String::from_utf8(bytes).ok().map(Cow::Owned),
        }
    }

    /// The next byte field, or `None` if it is unterminated or malformed.
    pub fn blob(&mut self) -> Option<Cow<'a, [u8]>> {
        let mut at = 0;
        let mut escaped = false;
        loop {
            let next = self.0[at..].iter().position(|byte| *byte == 0)? + at;
            match self.0.get(next + 1)? {
                0x00 => {
                    let field = &self.0[..next];
                    self.0 = &self.0[next + 2..];
                    return Some(if escaped {
                        Cow::Owned(unescape(field))
                    } else {
                        Cow::Borrowed(field)
                    });
                }
                0x01 => {
                    escaped = true;
                    at = next + 2;
                }
                // Neither a terminator nor an escape: this is not a key this module wrote.
                _ => return None,
            }
        }
    }

    /// Whether every field has been read.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// What is left, for a caller that knows the tail is not a field.
    pub fn rest(&self) -> &'a [u8] {
        self.0
    }
}

/// Undo [`escape`] over a field that is known to contain at least one escape.
fn unescape(field: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len());
    let mut bytes = field.iter();
    while let Some(byte) = bytes.next() {
        out.push(*byte);
        if *byte == 0 {
            // The `0x01` that follows an escaped NUL is not part of the value.
            bytes.next();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three shapes the contract names, so the round trip is tested on the real keys.
    fn run_key(dir: u64, mode: &str, argv: &str) -> Vec<u8> {
        Key::new().int(dir).text(mode).text(argv).done()
    }

    #[test]
    fn a_key_reads_back_as_the_fields_it_was_built_from() {
        let key = run_key(42, "sh", "cargo run --example xyz");
        let mut fields = Fields::of(&key);
        assert_eq!(fields.int(), Some(42));
        assert_eq!(fields.text().as_deref(), Some("sh"));
        assert_eq!(fields.text().as_deref(), Some("cargo run --example xyz"));
        assert!(fields.is_empty(), "and there is nothing left over");
    }

    /// A field that contains the separator is the case the encoding exists for. It must survive
    /// the round trip and it must not be able to forge a field boundary.
    #[test]
    fn a_field_containing_the_separator_survives_and_forges_nothing() {
        let key = Key::new().text("a\u{0}b").text("c").done();
        let mut fields = Fields::of(&key);
        assert_eq!(fields.text().as_deref(), Some("a\u{0}b"));
        assert_eq!(
            fields.text().as_deref(),
            Some("c"),
            "the NUL inside the first field did not end it"
        );
        assert!(fields.is_empty());

        // And the two are different keys, which a one-byte terminator would not guarantee.
        assert_ne!(key, Key::new().text("a").text("b").text("c").done());
    }

    /// The property everything else rests on: encoded order *is* logical order. Sorting the
    /// tuples and sorting their encodings must give the same sequence, over an alphabet chosen to
    /// hit the separator, the escape byte and the top of the byte range.
    #[test]
    fn the_encoded_bytes_sort_exactly_as_the_fields_do() {
        let words = [
            "", "\u{0}", "\u{1}", "a", "a\u{0}", "\u{0}a", "aa", "ab", "\u{ff}",
        ];
        let mut logical: Vec<(u64, &str, &str)> = Vec::new();
        for dir in [0u64, 1, 255, 256, u64::MAX] {
            for mode in ["sh", "lua"] {
                for argv in words {
                    logical.push((dir, mode, argv));
                }
            }
        }
        logical.sort();

        let mut encoded: Vec<Vec<u8>> = logical
            .iter()
            .map(|(dir, mode, argv)| run_key(*dir, mode, argv))
            .collect();
        let expected = encoded.clone();
        encoded.sort();
        assert_eq!(
            encoded, expected,
            "a key whose byte order does not match its field order breaks every prefix scan"
        );
    }

    /// The specific inversion a `0xFF` separator produces, pinned as a test so the argument in the
    /// module note is checkable rather than merely asserted. `cargo` is a prefix of `cargot`, so it
    /// must sort first — with a high separator it does not.
    #[test]
    fn a_prefix_field_sorts_before_the_field_that_extends_it() {
        assert!(run_key(1, "sh", "cargo") < run_key(1, "sh", "cargot"));
        assert!(
            Key::new().text("cargo").int(0).done() < Key::new().text("cargot").int(0).done(),
            "and still does when another field follows"
        );

        // What the rejected encoding would have done, spelled out.
        let high = |word: &str| {
            let mut bytes = word.as_bytes().to_vec();
            bytes.push(0xFF);
            bytes
        };
        assert!(
            high("cargo") > high("cargot"),
            "which is why the separator is not 0xFF"
        );
    }

    /// Big-endian, because a shell that sorted directory 256 before directory 2 would hand every
    /// range scan the wrong rows.
    #[test]
    fn integers_sort_as_numbers_rather_than_as_bytes() {
        assert!(Key::new().int(2).done() < Key::new().int(256).done());
        assert!(Key::new().int(255).done() < Key::new().int(256).done());
        assert!(Key::new().int(0).done() < Key::new().int(u64::MAX).done());
        assert_eq!(
            Key::new().int(1).done().len(),
            8,
            "fixed width, no terminator"
        );
    }

    /// A negative is a smaller number, not a larger bit pattern.
    #[test]
    fn a_signed_integer_sorts_below_a_positive_one() {
        let of = |n: i64| Key::new().signed(n).done();
        assert!(of(i64::MIN) < of(-1));
        assert!(of(-1) < of(0));
        assert!(of(0) < of(1));
        assert!(of(1) < of(i64::MAX));
        let negative = of(-7);
        assert_eq!(Fields::of(&negative).signed(), Some(-7));
    }

    /// A prefix key is the same bytes as the whole key up to where the field would have ended,
    /// which is the property that makes a prefix scan a range rather than a filter.
    #[test]
    fn a_prefix_is_the_head_of_every_key_it_should_find() {
        let prefix = Key::new().int(7).text("sh").text_prefix("cargo te").done();
        for line in ["cargo te", "cargo test", "cargo tesseract"] {
            assert!(
                run_key(7, "sh", line).starts_with(&prefix),
                "{line} starts with what was typed"
            );
        }
        for (dir, mode, line) in [
            (7, "sh", "cargo t"),
            (7, "sh", "cargo tf"),
            (7, "lua", "cargo test"),
            (8, "sh", "cargo test"),
        ] {
            assert!(
                !run_key(dir, mode, line).starts_with(&prefix),
                "{line} in {dir}/{mode} is outside the range"
            );
        }
    }

    /// A prefix that ends inside an escape is still the head of the keys it names — the escape is
    /// emitted whole, so there is no half-written separator for a scan to trip over.
    #[test]
    fn a_prefix_ending_in_the_separator_is_still_a_prefix() {
        let prefix = Key::new().text_prefix("a\u{0}").done();
        assert_eq!(prefix, b"a\x00\x01");
        assert!(Key::new().text("a\u{0}b").done().starts_with(&prefix));
        assert!(
            !Key::new().text("a").done().starts_with(&prefix),
            "and the shorter field, whose key is a 00 00, is not caught by it"
        );
    }

    /// Bytes this module did not write are refused rather than decoded into something plausible.
    #[test]
    fn a_key_from_somewhere_else_decodes_to_nothing() {
        assert_eq!(Fields::of(b"no terminator").blob(), None);
        assert_eq!(Fields::of(b"bad\x00\x07escape\x00\x00").blob(), None);
        assert_eq!(Fields::of(b"\x00").blob(), None, "cut off mid-terminator");
        assert_eq!(Fields::of(b"short").int(), None);
        // Valid framing, invalid text: a blob is fine, a string is not.
        let key = Key::new().blob(&[0xFF, 0xFE]).done();
        assert!(Fields::of(&key).blob().is_some());
        assert_eq!(Fields::of(&key).text(), None);
    }

    /// An empty field is a field, not an absent one.
    #[test]
    fn an_empty_field_is_still_there_when_it_is_read_back() {
        let key = Key::new().text("").text("x").done();
        let mut fields = Fields::of(&key);
        assert_eq!(fields.text().as_deref(), Some(""));
        assert_eq!(fields.text().as_deref(), Some("x"));
        assert!(fields.is_empty());
    }
}
