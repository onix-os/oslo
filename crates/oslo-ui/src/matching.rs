//! How a candidate is matched against what has been typed.
//!
//! A prefix test answers "does this start with what I typed". A **transform** answers "could what I
//! typed have been an abbreviation of this" — and that difference is most of why zsh's completion
//! feels like it is reading your mind. `/u/s/b` reaching `/usr/share/bin` and `f-b` reaching
//! `foo-bar` both come from here.
//!
//! Kept apart from the candidate builders because it is the one piece of completion that can be
//! reasoned about — and tested — without a filesystem, a `$PATH` or a terminal.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::cell::RefCell;

mod quality;
pub use quality::Quality;

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
    /// The typed letters appear in order with no gap wider than the preset allows, so `gco` reaches
    /// `git checkout`. Last in the chain and never mixed with the others.
    Fuzzy(Fuzzy),
}

/// Every way of matching that needs no configuration, in the order they are tried.
pub const MATCHERS: [Match; 3] = [Match::Exact, Match::Ignoring, Match::Pieces];

impl Match {
    /// Whether `candidate` matches `typed` this way.
    pub fn matches(self, candidate: &str, typed: &str) -> bool {
        match self {
            Match::Exact => candidate.starts_with(typed),
            Match::Ignoring => matches_ignoring_case(candidate, typed),
            Match::Pieces => matches_by_piece(candidate, typed),
            Match::Fuzzy(fuzzy) => fuzzy_score(candidate, typed, fuzzy).is_some(),
        }
    }
}

/// The matchers to try, in order, for a given fuzzy setting.
///
/// Fuzzy goes last and only runs when everything stricter came back empty, so switching it on can
/// never dilute a list that already had a real prefix match in it.
pub fn matchers(fuzzy: Fuzzy) -> Vec<Match> {
    let mut chain = MATCHERS.to_vec();
    if fuzzy != Fuzzy::Off {
        chain.push(Match::Fuzzy(fuzzy));
    }
    chain
}

/// How far apart the letters you typed may be scattered through a candidate.
///
/// A gap cap rather than a plain subsequence test, which is the difference between fuzzy matching
/// that helps and fuzzy matching that returns everything: unbounded, `cat` matches
/// `create_application_target` and every other word on the machine containing those three letters in
/// order. deja arrived at the same three presets and the same reasoning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fuzzy {
    /// No subsequence matching at all. What you type must prefix what you get.
    #[default]
    Off,
    /// Letters must be nearly adjacent. `gco` will not reach `git checkout`.
    Tight,
    /// Tuned so the canonical shell abbreviations work: `gco` reaches `git checkout`.
    Smart,
    /// Wide enough to feel like it is guessing, and it sometimes is.
    Loose,
}

impl Fuzzy {
    /// The pattern this preset scores with, or `None` when it matches nothing.
    ///
    /// **What the presets used to be was a cap on the gap between two typed letters**, and that cap
    /// is what made the search feel broken: at `Smart` it was four, so `cbf` could not reach
    /// `cargo build --features` — the very query a fuzzy finder exists to answer. A gap limit is a
    /// crude stand-in for ranking, and now that the score is a real one it is not needed.
    ///
    /// What the presets mean now is *how loosely a word may be spelled*:
    ///
    /// * `Tight` — the letters must be together. `argo` finds `cargo`, `cgo` does not.
    /// * `Smart` — fuzzy, and a capital in the query means a capital in the candidate.
    /// * `Loose` — fuzzy, and case is ignored entirely.
    ///
    /// Space separates *atoms*, each of which must match somewhere and in any order — so
    /// `push git` finds `git push`, which no single-subsequence matcher can do. fzf's own syntax
    /// comes with it: `'exact`, `^begins`, `ends$`, `!not`.
    fn pattern(self, typed: &str) -> Option<Pattern> {
        if typed.is_empty() {
            return None;
        }
        let (case, kind) = match self {
            Fuzzy::Off => return None,
            Fuzzy::Tight => (CaseMatching::Smart, AtomKind::Substring),
            Fuzzy::Smart => (CaseMatching::Smart, AtomKind::Fuzzy),
            Fuzzy::Loose => (CaseMatching::Ignore, AtomKind::Fuzzy),
        };
        Some(Pattern::new(typed, case, Normalization::Smart, kind))
    }

    /// Read a setting. `true` means the default preset, `false` means off.
    pub fn parse(name: &str) -> Option<Fuzzy> {
        match name {
            "off" | "false" | "none" => Some(Fuzzy::Off),
            "tight" => Some(Fuzzy::Tight),
            "smart" | "true" | "on" => Some(Fuzzy::Smart),
            "loose" => Some(Fuzzy::Loose),
            _ => None,
        }
    }

    /// The name this reads back as, for `oslo.settings` and the status line.
    pub fn name(self) -> &'static str {
        match self {
            Fuzzy::Off => "off",
            Fuzzy::Tight => "tight",
            Fuzzy::Smart => "smart",
            Fuzzy::Loose => "loose",
        }
    }

    /// The next preset in the ramp, for a key bound to cycle it.
    pub fn next(self) -> Fuzzy {
        match self {
            Fuzzy::Off => Fuzzy::Tight,
            Fuzzy::Tight => Fuzzy::Smart,
            Fuzzy::Smart => Fuzzy::Loose,
            Fuzzy::Loose => Fuzzy::Off,
        }
    }
}

/// How well `typed` matches `candidate` as a gap-capped subsequence, or `None` for no match.
///
/// Higher is better. The score exists because a fuzzy match without one is unusable: every
/// candidate that matches at all matches equally, so the list comes back in whatever order the
/// source happened to produce. What it rewards, in order of weight:
///
/// * letters that land at the start of a word — `gco` on `git checkout` hits `g`, `c`, `o` where a
///   human would point, and that is the match worth ranking first;
/// * runs of adjacent letters, so a near-prefix beats a scatter;
/// * an early first match, because `cargo` for `ca` should beat `libcargo`.
///
/// Case-insensitive, and folded per character rather than by lowercasing both strings — this runs
/// per candidate per keystroke, and a `String` each time is thousands of allocations.
pub fn fuzzy_score(candidate: &str, typed: &str, fuzzy: Fuzzy) -> Option<i32> {
    Fuzzed::new(typed, fuzzy).score(candidate)
}

/// A typed pattern, folded once, ready to be scored against many candidates.
///
/// **The point is the `once`.** [`fuzzy_score`] folds `typed` into a `Vec<char>` on every call, and
/// the caller is a loop over every executable on `$PATH` — measured at ~3,300 here, so the same
/// short pattern was being rebuilt 3,300 times per Tab press. Hoisting it out of the loop is the
/// whole optimisation; the scoring itself was never the cost.
///
/// The candidate still has to be folded per call, because it is different every time.
pub struct Fuzzed<'a> {
    /// The typed text, folded once — still needed by [`Quality`], which asks a different question
    /// from the scorer and answers it about the raw characters.
    wanted: Vec<char>,
    typed: &'a str,
    /// The pattern nucleo scores with, or `None` for `Fuzzy::Off` and an empty query.
    pattern: Option<Pattern>,
    /// nucleo scores through a reusable buffer and so takes `&mut self`; the six callers hold a
    /// `Fuzzed` by shared reference inside a loop, and a cell here is what keeps their code the
    /// shape it already had. Single-threaded by construction: this is drawn on one terminal.
    matcher: RefCell<Matcher>,
    /// Reused across candidates for the same reason the pattern is: one allocation per keystroke
    /// instead of one per candidate. `Utf32Str` needs somewhere to borrow from.
    haystack: RefCell<Vec<char>>,
}

impl<'a> Fuzzed<'a> {
    pub fn new(typed: &'a str, fuzzy: Fuzzy) -> Fuzzed<'a> {
        Fuzzed {
            wanted: typed.chars().flat_map(char::to_lowercase).collect(),
            typed,
            pattern: fuzzy.pattern(typed),
            matcher: RefCell::new(Matcher::new(Config::DEFAULT)),
            haystack: RefCell::new(Vec::new()),
        }
    }

    pub fn score(&self, candidate: &str) -> Option<i32> {
        // **Nothing typed matches everything**, and it is asked first: every widget draws its whole
        // list before a key is pressed, and a `Fuzzed` built from an empty query that answered
        // `None` would open each of them empty. `Fuzzy::Off` is the case that answers `None`.
        if self.typed.is_empty() {
            return Some(0);
        }
        let pattern = self.pattern.as_ref()?;
        let mut haystack = self.haystack.borrow_mut();
        let utf32 = Utf32Str::new(candidate, &mut haystack);
        let score = pattern.score(utf32, &mut self.matcher.borrow_mut())? as i32;
        // **Less left over wins a tie.** nucleo scores the match, not the leftovers, so `cargo` and
        // `cargo-nextest-runner` both answer 140 for `cargo` and the list then falls back to
        // whatever order it was built in. A small penalty, capped, so it settles ties without ever
        // outweighing a genuinely better match.
        Some(score - (utf32.len().saturating_sub(self.wanted.len())).min(40) as i32 / 4)
    }

    /// What kind of match this is, and how good a one.
    ///
    /// Two answers to two questions: [`Quality`] is the coarse kind a person can see and the
    /// finder sorts by, and the score is the tie-breaker within it. Only the score changed when the
    /// matcher did — the kinds were never the part that was wrong.
    pub fn rank(&self, candidate: &str) -> Option<(Quality, i32)> {
        if self.typed.is_empty() {
            return Some((Quality::Scattered, 0));
        }
        let score = self.score(candidate)?;
        let have: Vec<char> = candidate.chars().flat_map(char::to_lowercase).collect();
        Some((Quality::of(&have, &self.wanted), score))
    }
}

const SEPARATORS: [char; 8] = ['/', '-', '_', '.', ' ', ':', '@', '='];

fn starts_word(have: &[char], index: usize) -> bool {
    index == 0
        || index
            .checked_sub(1)
            .and_then(|before| have.get(before))
            .is_some_and(|c| SEPARATORS.contains(c))
}

/// Whether `wanted` is the initials of the candidate's words, in order and without skipping one.
///
/// `gco` is `git checkout origin`. `gio` is not, because it would have to skip `checkout` — and a
/// query that skips words is a scatter, which is what the last tier is for.
///
/// **A separator is not a word.** `cbr` on `cargo build --release` is the acronym anybody would
/// expect, and it was not one: the `-` of `--release` starts a "word" by the rule above, so the
/// initials read `c b - - r` and the third letter did not line up. In a shell almost every command
/// has a flag in it, so the tier that exists for `gco` was reached by almost nothing.
fn is_acronym(have: &[char], wanted: &[char]) -> bool {
    let mut initials = (0..have.len())
        .filter(|&i| starts_word(have, i) && have[i].is_alphanumeric())
        .map(|i| have[i]);
    wanted.iter().all(|&want| initials.next() == Some(want))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical shell abbreviation, and the reason `smart` is the default preset.
    #[test]
    fn gco_reaches_git_checkout_at_smart_but_not_at_tight() {
        assert!(fuzzy_score("git checkout", "gco", Fuzzy::Smart).is_some());
        assert!(
            fuzzy_score("git checkout", "gco", Fuzzy::Tight).is_none(),
            "tight means nearly adjacent"
        );
        assert!(
            fuzzy_score("git checkout", "gco", Fuzzy::Off).is_none(),
            "off must not match anything at all"
        );
    }

    /// **A scatter is ranked down, not refused** — which is the change of mind this matcher is.
    ///
    /// The old rule was a cap on the gap between two typed letters, and at `Smart` it was four: a
    /// query like `cbf` could not reach `cargo build --features` because the gaps are wider than
    /// that, and a fuzzy finder that cannot answer `cbf` is one nobody would call fuzzy. A cap is a
    /// crude stand-in for ranking; with a real score the sprawl simply scores badly, and
    /// [`Quality`] puts it in the last tier where the eye never reaches it.
    ///
    /// `Tight` is where "the letters must be together" still lives.
    #[test]
    fn a_wide_scatter_is_ranked_down_rather_than_refused() {
        let sprawl = "cxxxxxxxxxxaxxxxxxxxxxt";
        assert!(fuzzy_score(sprawl, "cat", Fuzzy::Smart).is_some());
        assert!(
            fuzzy_score("create_a_template", "cat", Fuzzy::Smart)
                > fuzzy_score(sprawl, "cat", Fuzzy::Smart),
            "tighter scores better"
        );
        assert!(
            fuzzy_score(sprawl, "cat", Fuzzy::Tight).is_none(),
            "tight is the preset that refuses a scatter"
        );
    }

    /// The query the whole change was for: letters that land on word starts, however far apart.
    #[test]
    fn an_acronym_reaches_across_a_command_line() {
        assert!(
            fuzzy_score("cargo build --release --all-features", "cbf", Fuzzy::Smart).is_some(),
            "the gap cap used to refuse this outright"
        );
        assert!(fuzzy_score("git commit --amend", "gca", Fuzzy::Smart).is_some());
    }

    /// Space separates atoms, and each may match anywhere — so the order you remember them in does
    /// not have to be the order they were typed in.
    #[test]
    fn words_may_be_given_in_any_order() {
        assert!(fuzzy_score("git push origin main", "push git", Fuzzy::Smart).is_some());
        assert!(fuzzy_score("git push origin main", "git push", Fuzzy::Smart).is_some());
        assert!(fuzzy_score("git pull origin main", "push git", Fuzzy::Smart).is_none());
    }

    /// The alignment a person means by an abbreviation, which greedy matching gets wrong.
    ///
    /// `cre` against `cargo run --example` has an `r` buried in `cargo` that a first-occurrence
    /// scan takes, stranding the final `e` nine characters away and refusing the whole candidate.
    /// Preferring a reachable word start finds `c`argo `r`un --`e`xample instead.
    #[test]
    fn a_word_start_is_preferred_over_an_earlier_letter_inside_a_word() {
        assert!(
            fuzzy_score("cargo run --example", "cre", Fuzzy::Loose).is_some(),
            "the obvious match must not be refused"
        );
        let boundary = fuzzy_score("cargo run --example", "cre", Fuzzy::Loose).unwrap();
        let buried = fuzzy_score("cargoruneexample", "cre", Fuzzy::Loose).unwrap();
        assert!(
            boundary > buried,
            "boundary {boundary} should beat buried {buried}"
        );
    }

    /// A near-prefix must beat a scatter, or the list fills with noise.
    #[test]
    fn adjacent_letters_beat_scattered_ones() {
        let together = fuzzy_score("cargo", "car", Fuzzy::Smart).unwrap();
        let apart = fuzzy_score("clear-archive", "car", Fuzzy::Smart).unwrap();
        assert!(together > apart, "{together} should beat {apart}");
    }

    /// Between two candidates that matched the same way, less left over wins.
    #[test]
    fn the_shorter_candidate_wins_a_tie() {
        let short = fuzzy_score("cargo", "cargo", Fuzzy::Smart).unwrap();
        let long = fuzzy_score("cargo-nextest-runner", "cargo", Fuzzy::Smart).unwrap();
        assert!(short > long, "{short} should beat {long}");
    }

    /// **Smart case, which is what the word has meant everywhere else for a decade**: a query typed
    /// in lower case ignores case, and a capital in the query asks for a capital. `Loose` is the
    /// preset for somebody who wants case ignored whatever they typed.
    #[test]
    fn a_capital_in_the_query_is_a_request() {
        assert!(fuzzy_score("README.md", "rdme", Fuzzy::Smart).is_some());
        assert!(fuzzy_score("README.md", "RDME", Fuzzy::Smart).is_some());
        assert!(
            fuzzy_score("readme.md", "RDME", Fuzzy::Smart).is_none(),
            "capitals were asked for and there are none"
        );
        assert!(fuzzy_score("readme.md", "RDME", Fuzzy::Loose).is_some());
    }

    #[test]
    fn a_preset_reads_back_as_what_it_was_written_as() {
        for name in ["off", "tight", "smart", "loose"] {
            assert_eq!(Fuzzy::parse(name).unwrap().name(), name);
        }
        assert_eq!(Fuzzy::parse("true"), Some(Fuzzy::Smart));
        assert_eq!(Fuzzy::parse("false"), Some(Fuzzy::Off));
        assert_eq!(Fuzzy::parse("nonsense"), None);
    }

    /// Cycling must come back to where it started, or a key bound to it walks into a corner.
    #[test]
    fn the_ramp_is_a_loop() {
        let mut at = Fuzzy::Off;
        for _ in 0..4 {
            at = at.next();
        }
        assert_eq!(at, Fuzzy::Off);
    }
}

/// Which bytes of `candidate` the user's `typed` characters landed on.
///
/// For painting: the finder marks the characters that matched, so a fuzzy hit shows *why* it is a
/// hit. Returns byte offsets, each the start of a matched character, in ascending order.
///
/// A contiguous run wins when there is one, because that is what a person reading the row expects
/// to see marked — `ec` in `echo` is one block, not two scattered letters. Failing that, the
/// leftmost subsequence is marked, which is the same walk the loose matchers make.
///
/// Empty when nothing matched, or when `typed` is empty: an empty query marks nothing rather than
/// everything, or opening the finder would light up the whole screen.
pub fn positions(candidate: &str, typed: &str) -> Vec<usize> {
    if typed.is_empty() || candidate.is_empty() {
        return Vec::new();
    }
    let lower_candidate = candidate.to_lowercase();
    let lower_typed = typed.to_lowercase();

    // A substring hit, marked as the one block it is.
    if let Some(at) = lower_candidate.find(&lower_typed) {
        // `to_lowercase` can change byte lengths, so the offset is re-derived by walking the
        // original — a `İ` earlier in the line would otherwise shift every mark after it.
        return contiguous_from(candidate, &lower_candidate, at, &lower_typed);
    }

    // Otherwise the leftmost subsequence.
    let mut wanted = lower_typed.chars().peekable();
    let mut found = Vec::new();
    for (at, ch) in candidate.char_indices() {
        let Some(&next) = wanted.peek() else { break };
        if ch.to_lowercase().next() == Some(next) {
            found.push(at);
            wanted.next();
        }
    }
    if wanted.peek().is_some() {
        // Not every typed character was placed, so this is not a match and marking part of it
        // would claim one.
        return Vec::new();
    }
    found
}

/// The byte offsets of a contiguous match found in the lowercased copy, mapped back to the
/// original.
fn contiguous_from(candidate: &str, lower: &str, at: usize, typed: &str) -> Vec<usize> {
    // How many characters precede the hit, and how many it spans. Counting in characters is what
    // makes this survive a case fold that changed the byte length.
    let before = lower[..at].chars().count();
    let span = typed.chars().count();
    candidate
        .char_indices()
        .skip(before)
        .take(span)
        .map(|(offset, _)| offset)
        .collect()
}

#[cfg(test)]
mod position_tests {
    use super::positions;

    /// A substring is marked as one block.
    #[test]
    fn a_substring_is_one_run() {
        assert_eq!(positions("echo hello", "ech"), vec![0, 1, 2]);
        assert_eq!(positions("git checkout", "check"), vec![4, 5, 6, 7, 8]);
    }

    /// Case does not matter to the match, and the marks land on the original's bytes.
    #[test]
    fn matching_ignores_case() {
        assert_eq!(positions("Cargo Build", "cargo"), vec![0, 1, 2, 3, 4]);
    }

    /// A scattered match marks each character it placed.
    #[test]
    fn a_subsequence_marks_each_character() {
        // `gco` inside `git checkout`: g(0), the first c(4), then the first o after it — which is
        // at 9, not 6. `git checkout` is g i t ␠ c h e c k o u t.
        assert_eq!(positions("git checkout", "gco"), vec![0, 4, 9]);
    }

    /// Nothing matched marks nothing — never a partial claim.
    #[test]
    fn a_failed_match_marks_nothing() {
        assert!(positions("echo", "zzz").is_empty());
        assert!(positions("echo", "eco2").is_empty());
        assert!(
            positions("echo", "").is_empty(),
            "an empty query marks nothing"
        );
        assert!(positions("", "x").is_empty());
    }

    /// Multi-byte characters keep their real offsets, so a mark cannot land mid-character.
    #[test]
    fn multibyte_offsets_are_real_byte_offsets() {
        let found = positions("héllo wörld", "wö");
        // `w` is at byte 7 (h=0, é=1..3, l, l, o, space), `ö` at 8.
        assert_eq!(found.len(), 2);
        for at in found {
            assert!(
                "héllo wörld".is_char_boundary(at),
                "{at} is not a character boundary"
            );
        }
    }
}
