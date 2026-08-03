//! Deciding which remembered directory a keyword meant.
//!
//! Pure arithmetic and string work — no database, no filesystem, no clock. The store hands rows
//! in, this says which one `cd` should jump to. Two decisions live here and nothing else does.
//!
//! # The curve, and why not zoxide's
//!
//! `visits / (1 + ln(1 + age_hours))`, the expression already in `interactive/spec/frecency.rs`,
//! so the completion ranker and the directory jump agree about what "best" means. That agreement
//! matters more than which curve it is.
//!
//! zoxide instead keeps a raw visit counter on disk and multiplies it at query time by one of four
//! constants picked from four age buckets — x4 under an hour, x2 under a day, x0.5 under a week,
//! x0.25 beyond. There is no half-life anywhere in it. Those buckets are discontinuous: two
//! directories a minute apart in age trade places the instant the older one crosses a boundary,
//! and a directory loses three quarters of its standing for no reason the user can perceive. The
//! step function is a 2011 shell-script artefact — fasd computes the same thing in awk — and a
//! shell holding an open database is not paying that constraint.
//!
//! Normalised to a fresh visit, the house curve is 0.591 at an hour, 0.237 at a day, 0.163 at a
//! week, 0.132 at a month, 0.099 at a year. Flatter than zoxide's inside the first day and steeper
//! past a month, which is the right bias for a shell: within a working session you want frequency
//! to decide, not the clock.
//!
//! # The cascade, and why the curve is not the ranking
//!
//! zoxide's four loudest open issues are one defect wearing four hats — the keywords are a pure
//! *filter* and take no part in the score, so the most-visited candidate wins however badly it
//! matched. `prust` (308 visits) beats `rust` (36) for the query `rust` (#956); `code3` beats
//! `code` (#247); the search string is asked to contribute to the score and does not (#260); and
//! the `src` of the project you are standing in loses to a `src` you visited more last month
//! (#929).
//!
//! So match quality is the primary key here and frecency only orders candidates that matched
//! equally well. [`Tier`] is that quality, ordered, and [`compare`] reads it first — which makes
//! the best-matching non-empty tier win outright. That is autojump's structure with the house
//! frecency ordering each rung.

use std::cmp::Ordering;

/// How many visits a directory needs before a keyword that did not name it exactly may reach it.
///
/// One mistyped `cd` would otherwise teleport you into the typo for ever.
const MIN_VISITS: i64 = 2;

/// The house frecency: visits, decayed by how long ago the last one was.
///
/// `now` and `last_visit` are epoch seconds, as `dir.last_visit` stores them.
///
/// This is the only ranking in oslo. It had a twin — `score_sql`, the same expression written out
/// for the database to evaluate — which existed so that the ordering done in storage and the
/// ordering done here could not drift apart. The store has no query language now and cannot
/// express an ordering at all, so the twin is gone and the drift it guarded against is impossible
/// rather than merely tested for. Anything that wants rows in an order calls this.
pub fn score(visits: i64, last_visit: i64, now: i64) -> f64 {
    if visits <= 0 {
        return 0.0;
    }
    // A clock that went backwards — an NTP step, or a row written by a machine running ahead of
    // this one over a shared home — reads as "just now". Left signed it would score *above* a
    // fresh visit, so a single bad timestamp would pin one directory at the top permanently.
    let age_hours = (now - last_visit).max(0) as f64 / 3600.0;
    visits as f64 / (1.0 + (1.0 + age_hours).ln())
}

/// How well a directory matched what was typed, worst first.
///
/// The ordering is the point: `Ord` on this enum is the primary sort key, so a candidate that
/// matched better wins outright over one that merely got visited more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// The keywords are in the path under the last-keyword rule, but the final component did not
    /// earn a better tier by itself — the query spans components, or an earlier keyword reaches
    /// into the last one.
    Path,
    /// The final component contains the last keyword.
    Contains,
    /// The final component starts with the last keyword.
    Prefix,
    /// The final component *is* the last keyword. `cd rust` means the directory called `rust`.
    Exact,
}

/// What was typed, folded to lower case once.
///
/// Case folding happens here rather than per candidate per query because `dir.base` is stored
/// already folded; a query folded once meets it without allocating per row.
#[derive(Debug, Clone, Default)]
pub struct Query {
    keywords: Vec<String>,
}

impl Query {
    /// Build a query from the words typed, dropping empty ones.
    pub fn new<I, S>(words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Query {
            keywords: words
                .into_iter()
                .map(|word| word.as_ref().to_lowercase())
                .filter(|word| !word.is_empty())
                .collect(),
        }
    }

    /// The single-keyword case, which is all `cd` has an arity for today.
    pub fn one(word: &str) -> Self {
        Query::new([word])
    }

    /// Whether there is nothing to match on. An empty query matches no directory at all, so a
    /// `cd ""` cannot be answered by a jump.
    pub fn is_empty(&self) -> bool {
        self.keywords.is_empty()
    }

    /// The keywords, folded.
    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }

    /// How well `path` answers this query, or `None` if it does not answer it at all.
    pub fn tier_of(&self, path: &str) -> Option<Tier> {
        let (last, earlier) = self.keywords.split_last()?;
        let lowered = path.to_lowercase();
        let (parent, base) = split_base(&lowered);

        // The three good tiers are all judgements about the *final* component, so the keywords
        // before the last one have to be satisfied by what comes before it. Reading them across
        // the boundary would let `/x/foobar` claim an exact match for `foo bar`.
        if !base.is_empty() && consumes(parent, earlier) {
            if base == last {
                return Some(Tier::Exact);
            }
            if base.starts_with(last.as_str()) {
                return Some(Tier::Prefix);
            }
            if base.contains(last.as_str()) {
                return Some(Tier::Contains);
            }
        }
        matches_path(&lowered, &self.keywords).then_some(Tier::Path)
    }
}

/// Whether the keywords appear in `path`, in order, with nothing but the final component after the
/// last one.
///
/// zoxide's rule, kept verbatim because it is right and because it is the one rule that stops
/// `cd cargo` landing you in `~/src/cargo-helpers/vendor/x`. Note it is not "the last keyword must
/// *be* the final component": after the rightmost occurrence of that keyword there may be no
/// separator, so `foo` matches `/a/foo` and `/a/foobar` but not `/foo/bar`.
///
/// `path` must already be folded.
fn matches_path(path: &str, keywords: &[String]) -> bool {
    let Some((last, earlier)) = keywords.split_last() else {
        return false;
    };
    let Some(at) = path.rfind(last.as_str()) else {
        return false;
    };
    if path[at + last.len()..].contains('/') {
        return false;
    }
    consumes(&path[..at], earlier)
}

/// Whether every keyword is present in `head`, in order, searching right to left.
///
/// Right to left so that the rightmost reading wins: the last keyword has already claimed the end
/// of the path, and each earlier one takes the latest place it still fits.
fn consumes(mut head: &str, keywords: &[String]) -> bool {
    for keyword in keywords.iter().rev() {
        match head.rfind(keyword.as_str()) {
            Some(at) => head = &head[..at],
            None => return false,
        }
    }
    true
}

/// A path split into everything up to its final component, and that component.
///
/// Trailing separators are ignored, so `/a/b/` splits like `/a/b`. The root has no final component
/// at all, which is what the empty second half means.
fn split_base(path: &str) -> (&str, &str) {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return (path, "");
    }
    match trimmed.rfind('/') {
        Some(at) => (&trimmed[..=at], &trimmed[at + 1..]),
        None => ("", trimmed),
    }
}

/// Whether a match is confident enough to jump to.
///
/// A directory seen once is reachable only by naming its final component exactly. Otherwise one
/// stray `cd` into a mistyped path teleports you there for ever, which is the failure that makes
/// people turn a jump tool off.
pub fn eligible(tier: Tier, visits: i64) -> bool {
    tier == Tier::Exact || visits >= MIN_VISITS
}

/// A remembered directory, as the ranker needs it. Built by the query layer from a `dir` row.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: String,
    pub visits: i64,
    /// Epoch seconds.
    pub last_visit: i64,
}

/// A candidate that matched, with everything the ordering reads.
#[derive(Debug, Clone)]
pub struct Ranked {
    pub path: String,
    pub tier: Tier,
    pub score: f64,
    /// Whether it lies inside the workspace the shell is standing in. See [`rank`].
    pub local: bool,
}

/// The ordering: best first.
///
/// Match quality, then locality, then frecency, then the shorter path — rupa/z's tie-break, which
/// zoxide dropped and #956 asks to have back. Path text settles the rest, because two candidates
/// equal in every ranking term must still come out in the same order on every run; a jump that
/// picks at random is worse than one that picks wrongly but predictably.
pub fn compare(a: &Ranked, b: &Ranked) -> Ordering {
    b.tier
        .cmp(&a.tier)
        .then_with(|| b.local.cmp(&a.local))
        .then_with(|| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal))
        .then_with(|| a.path.len().cmp(&b.path.len()))
        .then_with(|| a.path.cmp(&b.path))
}

/// Every candidate that matched and cleared the confidence floor, best first.
///
/// `workspace` is the project the shell is standing in — the git toplevel, or `None` outside a
/// repository. Candidates inside it outrank equally-matched candidates outside, which is the
/// answer to zoxide #929 and the same notion of "here" the directory-aware suggestion uses.
pub fn rank(
    candidates: impl IntoIterator<Item = Candidate>,
    query: &Query,
    now: i64,
    workspace: Option<&str>,
) -> Vec<Ranked> {
    let mut matched: Vec<Ranked> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let tier = query.tier_of(&candidate.path)?;
            if !eligible(tier, candidate.visits) {
                return None;
            }
            Some(Ranked {
                tier,
                local: within(&candidate.path, workspace),
                score: score(candidate.visits, candidate.last_visit, now),
                path: candidate.path,
            })
        })
        .collect();
    matched.sort_by(compare);
    matched
}

/// Where a jump should go, or `None` when nothing matched well enough.
pub fn best(
    candidates: impl IntoIterator<Item = Candidate>,
    query: &Query,
    now: i64,
    workspace: Option<&str>,
) -> Option<Ranked> {
    rank(candidates, query, now, workspace).into_iter().next()
}

/// Whether `path` is inside `workspace`, at a component boundary.
///
/// A workspace of `/` decides nothing, since every path is under it, so it counts as being
/// nowhere in particular.
fn within(path: &str, workspace: Option<&str>) -> bool {
    let Some(root) = workspace else {
        return false;
    };
    let root = root.trim_end_matches('/');
    !root.is_empty()
        && path
            .strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3600;
    const DAY: i64 = 24 * HOUR;
    const NOW: i64 = 1_700_000_000;

    fn seen(path: &str, visits: i64, ago: i64) -> Candidate {
        Candidate {
            path: path.to_string(),
            visits,
            last_visit: NOW - ago,
        }
    }

    fn jump(candidates: Vec<Candidate>, query: &str) -> Option<String> {
        best(candidates, &Query::one(query), NOW, None).map(|hit| hit.path)
    }

    /// The multipliers the design quotes, which are the whole argument for this curve over
    /// zoxide's four steps: a smooth slope with no cliff for two directories to trade places
    /// across.
    #[test]
    fn the_house_curve_decays_smoothly() {
        let fresh = score(1, NOW, NOW);
        assert!((fresh - 1.0).abs() < 1e-9, "a visit just now is worth 1.0");

        for (ago, expected) in [
            (HOUR, 0.591),
            (DAY, 0.237),
            (7 * DAY, 0.163),
            (30 * DAY, 0.132),
            (365 * DAY, 0.099),
        ] {
            let got = score(1, NOW - ago, NOW);
            assert!(
                (got - expected).abs() < 0.001,
                "{ago}s old scored {got}, expected about {expected}"
            );
        }

        // No cliff: a second either side of any age is a difference too small to reorder anything.
        for boundary in [HOUR, DAY, 7 * DAY] {
            let before = score(10, NOW - boundary + 1, NOW);
            let after = score(10, NOW - boundary - 1, NOW);
            assert!(before > after, "older is always worth less");
            assert!(
                (before - after).abs() < 0.001,
                "crossing {boundary}s must not move the score perceptibly"
            );
        }
    }

    #[test]
    fn visits_raise_the_score_and_age_lowers_it() {
        assert!(score(10, NOW - DAY, NOW) > score(3, NOW - DAY, NOW));
        assert!(score(10, NOW - HOUR, NOW) > score(10, NOW - 30 * DAY, NOW));
        assert_eq!(
            score(0, NOW, NOW),
            0.0,
            "a directory never visited is worth nothing"
        );
        // A timestamp from the future is read as now, not as a negative age that would outscore
        // every honest row.
        assert_eq!(score(4, NOW + 10 * DAY, NOW), score(4, NOW, NOW));
    }

    /// zoxide #956: `prust` had rank 308 and `rust` rank 36, and `z rust` went to `prust`. Naming
    /// a directory exactly is a stronger statement than having been there eight times as often.
    #[test]
    fn zoxide_956_an_exact_name_beats_a_frequent_partial() {
        let dirs = vec![
            seen("/home/u/prust", 308, HOUR),
            seen("/home/u/rust", 36, 30 * DAY),
        ];
        assert_eq!(jump(dirs, "rust"), Some("/home/u/rust".to_string()));
    }

    /// zoxide #247: `z code` preferred `code3` over `code`.
    #[test]
    fn zoxide_247_an_exact_name_beats_a_frequent_prefix() {
        let dirs = vec![
            seen("/home/u/code3", 400, HOUR),
            seen("/home/u/code", 20, 3 * DAY),
        ];
        assert_eq!(jump(dirs, "code"), Some("/home/u/code".to_string()));
    }

    /// zoxide #929: the `src` of the project you are standing in loses to a `src` you visited more
    /// last month. Both are exact matches, so no tier can separate them — locality does, and it
    /// is the same "which project am I in" the directory-aware suggestion already asks.
    #[test]
    fn zoxide_929_the_src_of_the_project_you_are_in_wins() {
        let here = "/home/u/work/api";
        let dirs = vec![
            seen("/home/u/work/api/src", 3, 2 * HOUR),
            seen("/home/u/other/src", 200, 30 * DAY),
        ];

        let landed = best(dirs.clone(), &Query::one("src"), NOW, Some(here));
        assert_eq!(
            landed.map(|hit| hit.path),
            Some("/home/u/work/api/src".to_string())
        );

        // Standing outside both projects, there is no locality to appeal to and frequency decides
        // — so it is the workspace doing the work above, not the arithmetic.
        let landed = best(dirs, &Query::one("src"), NOW, Some("/home/u/elsewhere"));
        assert_eq!(
            landed.map(|hit| hit.path),
            Some("/home/u/other/src".to_string())
        );
    }

    /// zoxide #260: the search string should take part in the score. It does — but only to the
    /// extent that it separates the candidates. Where they all match equally well, frequency is
    /// exactly the right decider and nothing here should override it.
    #[test]
    fn zoxide_260_frequency_still_decides_inside_a_tier() {
        let dirs = vec![
            seen("/home/u/cocoa", 5, HOUR),
            seen("/home/u/code", 40, HOUR),
            seen("/home/u/code3", 12, HOUR),
        ];
        // Nothing is named `co`, so all three are prefix matches and none outmatched the others.
        assert_eq!(jump(dirs, "co"), Some("/home/u/code".to_string()));
    }

    #[test]
    fn the_tiers_are_read_off_the_final_component() {
        let query = Query::one("rust");
        assert_eq!(query.tier_of("/home/u/rust"), Some(Tier::Exact));
        assert_eq!(query.tier_of("/home/u/rustlings"), Some(Tier::Prefix));
        assert_eq!(query.tier_of("/home/u/prust"), Some(Tier::Contains));
        assert_eq!(query.tier_of("/home/u/go/src"), None);
        // A parent is not a match at all: the last keyword has to land in the final component,
        // which is what keeps `cd rust` out of `/home/u/rust/book`.
        assert_eq!(query.tier_of("/home/u/rust/book"), None);
        // So the lowest tier is reached only by a query that spans components itself.
        assert_eq!(
            Query::one("u/rust").tier_of("/home/u/rust"),
            Some(Tier::Path)
        );
        // A trailing separator does not change which component is final.
        assert_eq!(query.tier_of("/home/u/rust/"), Some(Tier::Exact));
    }

    /// The last-keyword rule, which is what stops `cd cargo` landing in
    /// `~/src/cargo-helpers/vendor/x`. Not "the last keyword must *be* the final component": after
    /// its rightmost occurrence there may be no separator.
    #[test]
    fn the_last_keyword_must_land_in_the_final_component() {
        let table: &[(&[&str], &str, bool)] = &[
            (&["foo"], "/a/foo", true),
            (&["foo"], "/a/foobar", true),
            (&["foo"], "/foo/bar", false),
            (&["fOo", "bAr"], "/foo/bar", true),
            (&["ba"], "/foo/bar", true),
            (&["fo"], "/foo/bar", false),
            (&["foo/"], "/foo", false),
            (&["foo/"], "/foo/bar", true),
            (&["foo/"], "/foo/bar/baz", false),
            (&["foo", "/"], "/foo/bar", true),
            (&["/foo/", "bar"], "/foo/baz/bar", true),
            (&["foo", "bar"], "/foo/baz/bar", true),
            (&["foo", "bar"], "/test/foo/bar", true),
            (&["foo", "bar", "baz"], "/foo/bar/baz", true),
            // Order is part of the query: the keywords have to appear left to right.
            (&["bar", "foo"], "/foo/bar", false),
        ];
        for (keywords, path, expected) in table {
            let matched = Query::new(*keywords).tier_of(path).is_some();
            assert_eq!(matched, *expected, "{keywords:?} against {path}");
        }
    }

    #[test]
    fn case_is_folded_on_both_sides() {
        assert_eq!(
            Query::one("RUST").tier_of("/home/U/Rust"),
            Some(Tier::Exact)
        );
    }

    /// One mistyped `cd` must not become a permanent destination.
    #[test]
    fn a_directory_seen_once_is_reachable_only_by_its_exact_name() {
        let typo = vec![seen("/home/u/prjects", 1, HOUR)];
        assert_eq!(jump(typo.clone(), "pr"), None, "a prefix will not reach it");
        assert_eq!(jump(typo.clone(), "jec"), None, "nor will a substring");
        assert_eq!(
            jump(typo, "prjects"),
            Some("/home/u/prjects".to_string()),
            "naming it exactly still works — it is a real directory"
        );

        // Two visits is enough for the weaker tiers: it is a habit now, not a slip.
        assert_eq!(
            jump(vec![seen("/home/u/projects", 2, HOUR)], "pro"),
            Some("/home/u/projects".to_string())
        );
    }

    /// `cd ""` must not be answered by a jump to whatever happens to rank highest.
    #[test]
    fn an_empty_query_matches_nothing() {
        let query = Query::new(Vec::<String>::new());
        assert!(query.is_empty());
        assert_eq!(query.tier_of("/home/u/anything"), None);
        assert!(rank(vec![seen("/home/u/anything", 99, 0)], &query, NOW, None).is_empty());
        // A query made only of empty words is the same thing.
        assert!(Query::new(["", ""]).is_empty());
    }

    /// rupa/z's tie-break, which zoxide dropped: between two equally good, equally used matches,
    /// the shallower one is the one you meant.
    #[test]
    fn the_shorter_path_breaks_a_tie() {
        let dirs = vec![
            seen("/home/u/a/very/deep/place/src", 5, HOUR),
            seen("/home/u/src", 5, HOUR),
        ];
        assert_eq!(jump(dirs, "src"), Some("/home/u/src".to_string()));
    }

    /// The whole ordering in one go: a better tier outranks locality, and locality outranks
    /// frecency however lopsided the visit counts are.
    #[test]
    fn rank_orders_by_quality_then_locality_then_frecency() {
        let dirs = vec![
            seen("/w/other/notes", 500, HOUR),
            seen("/w/proj/notes", 4, 20 * DAY),
            seen("/w/other/notesbook", 900, HOUR),
            seen("/w/other/mynotes", 900, HOUR),
        ];
        let order: Vec<String> = rank(dirs, &Query::one("notes"), NOW, Some("/w/proj"))
            .into_iter()
            .map(|hit| hit.path)
            .collect();
        assert_eq!(
            order,
            vec![
                "/w/proj/notes",      // exact, and where I am standing
                "/w/other/notes",     // exact
                "/w/other/notesbook", // prefix
                "/w/other/mynotes",   // contains
            ]
        );
    }

    /// A match only the whole path could satisfy ranks under one the final component earned by
    /// itself, however much more often it was visited.
    #[test]
    fn a_path_only_match_ranks_last() {
        let dirs = vec![
            // `foo` and `bar` are both here, but only by overlapping inside one component.
            seen("/x/foobar", 900, HOUR),
            seen("/foo/bar", 3, 10 * DAY),
        ];
        let order: Vec<String> = rank(dirs, &Query::new(["foo", "bar"]), NOW, None)
            .into_iter()
            .map(|hit| hit.path)
            .collect();
        assert_eq!(order, vec!["/foo/bar", "/x/foobar"]);
    }

    #[test]
    fn a_workspace_is_matched_at_a_component_boundary() {
        assert!(within("/home/u/work/api/src", Some("/home/u/work/api")));
        assert!(within("/home/u/work/api", Some("/home/u/work/api")));
        assert!(within("/home/u/work/api/src", Some("/home/u/work/api/")));
        assert!(!within("/home/u/work/apiary", Some("/home/u/work/api")));
        assert!(!within("/home/u/other", Some("/home/u/work/api")));
        assert!(!within("/home/u/other", None));
        // Everything is under the root, so a root workspace tells the ranking nothing.
        assert!(!within("/home/u/other", Some("/")));
    }
}
