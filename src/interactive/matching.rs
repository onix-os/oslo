//! How a candidate is matched against what has been typed.
//!
//! A prefix test answers "does this start with what I typed". A **transform** answers "could what I
//! typed have been an abbreviation of this" — and that difference is most of why zsh's completion
//! feels like it is reading your mind. `/u/s/b` reaching `/usr/share/bin` and `f-b` reaching
//! `foo-bar` both come from here.
//!
//! Kept apart from the candidate builders because it is the one piece of completion that can be
//! reasoned about — and tested — without a filesystem, a `$PATH` or a terminal.

/// A prefix match that ignores case.
///
/// Folded per character rather than by lowercasing both strings: a `String` per candidate would be
/// thousands of allocations per keystroke on a large `$PATH`.
pub fn matches_ignoring_case(candidate: &str, typed: &str) -> bool {
    let mut wanted = typed.chars().flat_map(char::to_lowercase);
    let mut have = candidate.chars().flat_map(char::to_lowercase);
    loop {
        match (wanted.next(), have.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(a), Some(b)) if a == b => {}
            (Some(_), Some(_)) => return false,
        }
    }
}

/// How a candidate may be matched, tried in order until one finds something.
///
/// **This is most of why zsh's completion feels different.** A prefix test answers "does this start
/// with what I typed"; a *transform* answers "could what I typed have been an abbreviation of
/// this". `/u/s/b` reaching `/usr/share/bin` and `f-b` reaching `foo-bar` both come from here.
///
/// Ordered and first-non-empty-wins, deliberately. Trying them all at once and merging is how a
/// completion list fills with fuzzy noise when an exact match was sitting right there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// What was typed, exactly.
    Exact,
    /// The same, ignoring case — so `re` reaches `README.md`.
    Ignoring,
    /// Each separator-delimited piece is a prefix of the corresponding piece of the candidate, so
    /// `/u/s/b` reaches `/usr/share/bin` and `f-b` reaches `foo-bar`.
    Pieces,
}

/// Every way of matching, in the order they are tried.
pub const MATCHERS: [Match; 3] = [Match::Exact, Match::Ignoring, Match::Pieces];

impl Match {
    /// Whether `candidate` matches `typed` this way.
    pub fn matches(self, candidate: &str, typed: &str) -> bool {
        match self {
            Match::Exact => candidate.starts_with(typed),
            Match::Ignoring => matches_ignoring_case(candidate, typed),
            Match::Pieces => matches_by_piece(candidate, typed),
        }
    }
}

/// Whether every piece of `typed` prefixes the corresponding piece of `candidate`.
///
/// The separators are the ones that divide a name into parts a person thinks in: path components,
/// and the dashes, dots and underscores that make up a compound word. A piece that is empty matches
/// anything, so a doubled separator does not refuse the whole candidate.
fn matches_by_piece(candidate: &str, typed: &str) -> bool {
    const SEPARATORS: [char; 4] = ['/', '-', '_', '.'];
    // Only worth trying when the user actually wrote a separator; otherwise this is a plain prefix
    // test that has already been tried and failed.
    if !typed.contains(SEPARATORS) {
        return false;
    }
    let wanted: Vec<&str> = typed.split(SEPARATORS).collect();
    let have: Vec<&str> = candidate.split(SEPARATORS).collect();
    if wanted.len() > have.len() {
        return false;
    }
    // The last typed piece may still be being written, so it prefixes rather than equals — but the
    // ones before it are complete words the user has moved past.
    wanted
        .iter()
        .zip(have.iter())
        .enumerate()
        .all(|(i, (w, h))| {
            if w.is_empty() {
                return true;
            }
            if i + 1 == wanted.len() {
                matches_ignoring_case(h, w)
            } else {
                matches_ignoring_case(h, w) && (h.len() == w.len() || i + 1 < wanted.len())
            }
        })
}
