//! Half-open ranges, and the successor that makes a prefix into one.
//!
//! Every read in this store is a range: "what did I run *here* that starts like this" is a scan
//! from one key to another, not a scan of everything followed by a filter. The distinction is the
//! single most performance-relevant decision in `src/track/`, and it survives the move off SQL
//! unchanged — `argv LIKE 'cargo run --ex%'` and `for kv in bucket { if kv.starts_with(..) }` are
//! the same mistake in two languages, and both are O(rows in the bucket).
//!
//! # The successor
//!
//! [`upper_bound`] is [`super::super::db::upper_bound`] moved from characters to bytes: the
//! smallest key that sorts above everything beginning with `prefix`. Stepping the last byte is
//! exactly right where stepping the last *character* was, because the keys are compared as bytes
//! and UTF-8 byte order agrees with code point order — and it answers where the character version
//! could not, because there is a byte above `0xF4 0x8F 0xBF 0xBF` and no character above
//! `U+10FFFF`.
//!
//! `None` still means "there is nothing above this", which is an empty prefix or one that is all
//! `0xFF`. A caller reads that as "unbounded above" rather than "no answer": every key in the
//! bucket does begin with the empty prefix.

use std::ops::Bound;

/// The exclusive upper end of the keys beginning with `prefix`.
///
/// `None` when there is no such key — when every extension of `prefix` is also below every
/// possible successor, which is the case for an empty prefix and for one made entirely of `0xFF`.
pub fn upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut bound = prefix.to_vec();
    while let Some(last) = bound.pop() {
        if last != 0xFF {
            bound.push(last + 1);
            return Some(bound);
        }
    }
    None
}

/// A half-open span of keys: `lower` is included, `upper` is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    lower: Vec<u8>,
    /// `None` is unbounded — to the end of the bucket.
    upper: Option<Vec<u8>>,
}

impl Span {
    /// Every key beginning with `prefix`, which is what a composite-key scan is.
    ///
    /// Built from [`upper_bound`], so an all-`0xFF` prefix widens to "everything from here on"
    /// rather than answering nothing: those are the same set.
    pub fn prefix(prefix: Vec<u8>) -> Span {
        let upper = upper_bound(&prefix);
        Span {
            lower: prefix,
            upper,
        }
    }

    /// `lower ..= the end of the bucket`.
    pub fn from(lower: Vec<u8>) -> Span {
        Span { lower, upper: None }
    }

    /// `lower .. upper`, half-open. An `upper` at or below `lower` is an empty span, which is what
    /// it says rather than an error.
    pub fn between(lower: Vec<u8>, upper: Vec<u8>) -> Span {
        Span {
            lower,
            upper: Some(upper),
        }
    }

    /// Every key in the bucket, in order.
    pub fn all() -> Span {
        Span {
            lower: Vec::new(),
            upper: None,
        }
    }

    /// Whether `key` is inside the span. The scan does not need this — it stops at the bound — but
    /// the tests do, and so does anything that has a key already in hand.
    pub fn holds(&self, key: &[u8]) -> bool {
        key >= self.lower.as_slice()
            && self
                .upper
                .as_ref()
                .is_none_or(|upper| key < upper.as_slice())
    }

    /// The span as the engine's iterator wants it: seek to `lower`, walk until `upper`.
    ///
    /// One type for both ends on purpose — an unbounded upper end and a real one have to be the
    /// same `Bound`, or the caller needs two loops to iterate one span.
    pub(super) fn bounds(&self) -> (Bound<&[u8]>, Bound<&[u8]>) {
        (
            Bound::Included(self.lower.as_slice()),
            match &self.upper {
                Some(upper) => Bound::Excluded(upper.as_slice()),
                None => Bound::Unbounded,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::Key;
    use super::*;

    /// The same cases `db::upper_bound` is tested on, in the bytes the store actually compares.
    #[test]
    fn the_successor_names_the_first_key_above_the_prefix() {
        assert_eq!(
            upper_bound(b"cargo run --ex").as_deref(),
            Some(&b"cargo run --ey"[..])
        );
        assert_eq!(upper_bound(b"ab").as_deref(), Some(&b"ac"[..]));
        assert_eq!(upper_bound(b""), None);
        // A trailing 0xFF carries: the next key above `a\xFF` is `b`, not `a\x100`.
        assert_eq!(upper_bound(b"a\xff").as_deref(), Some(&b"b"[..]));
        assert_eq!(upper_bound(b"a\xff\xff").as_deref(), Some(&b"b"[..]));
        assert_eq!(upper_bound(b"\xff\xff"), None, "nothing sorts above this");
        // Where the character version had to give up, because U+10FFFF is the last character but
        // 0xBF is not the last byte.
        assert_eq!(
            upper_bound("\u{10FFFF}".as_bytes()).as_deref(),
            Some(&b"\xf4\x8f\xbf\xc0"[..])
        );
    }

    /// A prefix span holds exactly what starts with the prefix — including the prefix itself, and
    /// including the key that sits immediately below the top of the range.
    #[test]
    fn a_prefix_span_holds_the_extensions_and_stops_at_the_next_key() {
        let span = Span::prefix(b"cargo te".to_vec());
        for inside in [
            "cargo te",
            "cargo test",
            "cargo tesseract",
            "cargo te\u{ff}",
        ] {
            assert!(span.holds(inside.as_bytes()), "{inside}");
        }
        for outside in ["cargo t", "cargo tf", "cargo", "make cargo test"] {
            assert!(!span.holds(outside.as_bytes()), "{outside}");
        }
    }

    /// A prefix with nothing above it is every key from there on, not no keys at all.
    #[test]
    fn a_prefix_at_the_top_of_the_key_space_is_unbounded_rather_than_empty() {
        let span = Span::prefix(b"\xff\xff".to_vec());
        assert!(span.holds(b"\xff\xff"));
        assert!(span.holds(b"\xff\xff\xff"));
        assert!(!span.holds(b"\xff\xfe"));
        assert!(matches!(span.bounds().1, Bound::Unbounded));

        // And the empty prefix is the whole bucket, which is what `all` is.
        assert_eq!(Span::prefix(Vec::new()), Span::all());
    }

    /// The end is excluded, which is the half of "half-open" that is easy to get wrong.
    #[test]
    fn the_upper_end_is_outside_the_span() {
        let span = Span::between(b"b".to_vec(), b"d".to_vec());
        assert!(span.holds(b"b"));
        assert!(span.holds(b"c"));
        assert!(!span.holds(b"d"));
        assert!(!span.holds(b"a"));
        assert!(matches!(span.bounds().0, Bound::Included(b"b")));
        assert!(matches!(span.bounds().1, Bound::Excluded(b"d")));
    }

    /// The two spans the contract asks for, expressed against real composite keys: the run scan is
    /// pinned to one directory and one language, and the directory scan to one folded name.
    #[test]
    fn a_composite_prefix_span_stays_inside_its_directory() {
        let span = Span::prefix(Key::new().int(7).text("sh").text_prefix("cargo te").done());
        let key =
            |dir: u64, mode: &str, argv: &str| Key::new().int(dir).text(mode).text(argv).done();

        assert!(span.holds(&key(7, "sh", "cargo test")));
        assert!(!span.holds(&key(7, "sh", "cargo t")));
        assert!(!span.holds(&key(7, "lua", "cargo test")));
        assert!(!span.holds(&key(8, "sh", "cargo test")));
        assert!(
            !span.holds(&key(6, "sh", "cargo test")),
            "and the directory below it is outside too, not merely the one above"
        );
    }
}
