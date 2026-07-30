//! Lua patterns.
//!
//! Not regular expressions. Lua has its own matcher — `%a` rather than `\w`, `-` for a lazy
//! quantifier, no alternation at all — and scripts depend on the differences. `%b()` matches
//! balanced parentheses, which no regex engine does; `%f[%w]` is a frontier assertion; a pattern
//! with `+` in it means a literal plus in a regex dialect and a quantifier here.
//!
//! So this is a direct translation of the backtracking matcher in Lua's own `lstrlib.c`, kept
//! close to the original on purpose. Substituting a regex crate would be less code and would
//! silently change what every `string.gsub` in every script does.

/// Lua's own limit, and the size of the capture array.
const MAX_CAPTURES: usize = 32;

/// Bound on recursive backtracking, so a pathological pattern errors rather than overflowing the
/// Rust stack. A shell may not abort.
const MAX_DEPTH: usize = 200;

/// A capture is still open — the matcher has seen `(` but not the matching `)`.
const CAP_UNFINISHED: isize = -1;
/// A position capture, `()`, which yields an index rather than text.
const CAP_POSITION: isize = -2;

/// What one capture produced.
#[derive(Debug, Clone, PartialEq)]
pub enum Capture {
    Text(Vec<u8>),
    /// A 1-based index, from `()`.
    Position(usize),
}

/// A successful match: where it landed, and what it captured.
#[derive(Debug, Clone)]
pub struct Match {
    /// Byte offset of the first matched byte.
    pub start: usize,
    /// Byte offset one past the last matched byte.
    pub end: usize,
    /// Explicit captures, in order. Empty when the pattern has none — callers substitute the
    /// whole match, which is what `string.match("abc", "%a+")` returning `abc` means.
    pub captures: Vec<Capture>,
}

impl Match {
    /// The captures, or the whole match when the pattern declared none.
    pub fn captures_or_whole(&self, src: &[u8]) -> Vec<Capture> {
        if self.captures.is_empty() {
            vec![Capture::Text(src[self.start..self.end].to_vec())]
        } else {
            self.captures.clone()
        }
    }
}

struct Matcher<'a> {
    src: &'a [u8],
    pat: &'a [u8],
    /// `(start, len)` per capture, where `len` may be one of the two sentinels above.
    capture: Vec<(usize, isize)>,
    depth: usize,
}

type MatchResult = Result<Option<usize>, String>;

impl<'a> Matcher<'a> {
    fn new(src: &'a [u8], pat: &'a [u8]) -> Self {
        Matcher {
            src,
            pat,
            capture: Vec::new(),
            depth: 0,
        }
    }

    /// Pattern byte at `i`, or NUL past the end.
    ///
    /// The C original reads the string terminator here and branches on it; reproducing that is
    /// simpler and less error-prone than adding a bounds test to each of the dozen call sites.
    fn pat_at(&self, i: usize) -> u8 {
        self.pat.get(i).copied().unwrap_or(0)
    }

    /// Subject byte at `i`, or NUL past the end. Same reasoning as [`Matcher::pat_at`].
    fn src_at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    /// Index just past the single-character class starting at `p`.
    fn class_end(&self, mut p: usize) -> Result<usize, String> {
        let c = self.pat_at(p);
        p += 1;
        if c == b'%' {
            if p >= self.pat.len() {
                return Err("malformed pattern (ends with '%')".into());
            }
            return Ok(p + 1);
        }
        if c == b'[' {
            if self.pat_at(p) == b'^' {
                p += 1;
            }
            // At least one byte is consumed before `]` is looked for, which is what makes the `]`
            // in `[]]` a literal member of the set rather than its terminator.
            loop {
                if p >= self.pat.len() {
                    return Err("malformed pattern (missing ']')".into());
                }
                let ch = self.pat_at(p);
                p += 1;
                if ch == b'%' && p < self.pat.len() {
                    p += 1;
                }
                if self.pat_at(p) == b']' && p < self.pat.len() {
                    return Ok(p + 1);
                }
            }
        }
        Ok(p)
    }

    /// Whether `c` belongs to the set `[...]` spanning `p..=ec`.
    fn match_bracket_class(&self, c: u8, mut p: usize, ec: usize) -> bool {
        let mut sig = true;
        if self.pat_at(p + 1) == b'^' {
            sig = false;
            p += 1;
        }
        p += 1;
        while p < ec {
            if self.pat_at(p) == b'%' {
                p += 1;
                if match_class(c, self.pat_at(p)) {
                    return sig;
                }
                p += 1;
            } else if self.pat_at(p + 1) == b'-' && p + 2 < ec {
                // A range, `a-z`. Note this is checked before the plain-byte case, so a literal
                // `-` only survives at the very start or end of the set.
                p += 2;
                if self.pat_at(p - 2) <= c && c <= self.pat_at(p) {
                    return sig;
                }
                p += 1;
            } else {
                if self.pat_at(p) == c {
                    return sig;
                }
                p += 1;
            }
        }
        !sig
    }

    /// Whether the one class at `p..ep` matches the subject byte at `s`.
    fn single_match(&self, s: usize, p: usize, ep: usize) -> bool {
        if s >= self.src.len() {
            return false;
        }
        let c = self.src[s];
        match self.pat_at(p) {
            b'.' => true,
            b'%' => match_class(c, self.pat_at(p + 1)),
            b'[' => self.match_bracket_class(c, p, ep - 1),
            other => other == c,
        }
    }

    /// Match `pat[p..]` against `src[s..]`, returning where the match ended.
    fn do_match(&mut self, s: usize, p: usize) -> MatchResult {
        if self.depth >= MAX_DEPTH {
            return Err("pattern too complex".into());
        }
        self.depth += 1;
        let out = self.run(s, p);
        self.depth -= 1;
        out
    }

    fn run(&mut self, mut s: usize, mut p: usize) -> MatchResult {
        loop {
            if p >= self.pat.len() {
                return Ok(Some(s));
            }
            match self.pat_at(p) {
                b'(' => {
                    return if self.pat_at(p + 1) == b')' {
                        self.start_capture(s, p + 2, CAP_POSITION)
                    } else {
                        self.start_capture(s, p + 1, CAP_UNFINISHED)
                    };
                }
                b')' => return self.end_capture(s, p + 1),
                // `$` is an anchor only at the very end of the pattern; anywhere else it is a
                // literal dollar sign, which is why `"a$b"` matches the text `a$b`.
                b'$' if p + 1 == self.pat.len() => {
                    return Ok(if s == self.src.len() { Some(s) } else { None });
                }
                b'%' => match self.pat_at(p + 1) {
                    b'b' => match self.match_balance(s, p + 2)? {
                        Some(next) => {
                            s = next;
                            p += 4;
                            continue;
                        }
                        None => return Ok(None),
                    },
                    b'f' => {
                        p += 2;
                        if self.pat_at(p) != b'[' {
                            return Err("missing '[' after '%f' in pattern".into());
                        }
                        let ep = self.class_end(p)?;
                        // A frontier matches *between* bytes: the one before must be outside the
                        // set and the one at `s` inside it. Before the first byte, the previous
                        // one is treated as NUL, so `%f[%w]` fires at the start of the subject.
                        let previous = if s == 0 { 0 } else { self.src[s - 1] };
                        if !self.match_bracket_class(previous, p, ep - 1)
                            && self.match_bracket_class(self.src_at(s), p, ep - 1)
                        {
                            p = ep;
                            continue;
                        }
                        return Ok(None);
                    }
                    d if d.is_ascii_digit() => match self.match_capture(s, d)? {
                        Some(next) => {
                            s = next;
                            p += 2;
                            continue;
                        }
                        None => return Ok(None),
                    },
                    _ => {}
                },
                _ => {}
            }

            // An ordinary single-character class, possibly quantified.
            let ep = self.class_end(p)?;
            if !self.single_match(s, p, ep) {
                let quantifier = self.pat_at(ep);
                if matches!(quantifier, b'*' | b'?' | b'-') {
                    // Zero repetitions is a match; skip the class and carry on.
                    p = ep + 1;
                    continue;
                }
                return Ok(None);
            }
            match self.pat_at(ep) {
                b'?' => {
                    if let Some(res) = self.do_match(s + 1, ep + 1)? {
                        return Ok(Some(res));
                    }
                    p = ep + 1;
                }
                b'+' => return self.max_expand(s + 1, p, ep),
                b'*' => return self.max_expand(s, p, ep),
                b'-' => return self.min_expand(s, p, ep),
                _ => {
                    s += 1;
                    p = ep;
                }
            }
        }
    }

    /// Greedy repetition: take as many as possible, then give them back one at a time.
    fn max_expand(&mut self, s: usize, p: usize, ep: usize) -> MatchResult {
        let mut i = 0usize;
        while self.single_match(s + i, p, ep) {
            i += 1;
        }
        loop {
            if let Some(res) = self.do_match(s + i, ep + 1)? {
                return Ok(Some(res));
            }
            if i == 0 {
                return Ok(None);
            }
            i -= 1;
        }
    }

    /// Lazy repetition, spelled `-` in Lua rather than `*?`.
    fn min_expand(&mut self, mut s: usize, p: usize, ep: usize) -> MatchResult {
        loop {
            if let Some(res) = self.do_match(s, ep + 1)? {
                return Ok(Some(res));
            }
            if self.single_match(s, p, ep) {
                s += 1;
            } else {
                return Ok(None);
            }
        }
    }

    fn start_capture(&mut self, s: usize, p: usize, what: isize) -> MatchResult {
        if self.capture.len() >= MAX_CAPTURES {
            return Err("too many captures".into());
        }
        self.capture.push((s, what));
        let res = self.do_match(s, p)?;
        if res.is_none() {
            // The rest of the pattern failed, so this capture never happened.
            self.capture.pop();
        }
        Ok(res)
    }

    fn end_capture(&mut self, s: usize, p: usize) -> MatchResult {
        let Some(l) = self
            .capture
            .iter()
            .rposition(|(_, len)| *len == CAP_UNFINISHED)
        else {
            return Err("invalid pattern capture".into());
        };
        self.capture[l].1 = (s - self.capture[l].0) as isize;
        let res = self.do_match(s, p)?;
        if res.is_none() {
            self.capture[l].1 = CAP_UNFINISHED;
        }
        Ok(res)
    }

    /// `%bxy` — a balanced run from `x` to its matching `y`, counting nesting.
    fn match_balance(&mut self, s: usize, p: usize) -> MatchResult {
        if p + 1 >= self.pat.len() {
            return Err("malformed pattern (missing arguments to '%b')".into());
        }
        let (open, close) = (self.pat_at(p), self.pat_at(p + 1));
        if self.src_at(s) != open {
            return Ok(None);
        }
        let mut depth = 1;
        let mut i = s + 1;
        while i < self.src.len() {
            if self.src[i] == close {
                depth -= 1;
                if depth == 0 {
                    return Ok(Some(i + 1));
                }
            } else if self.src[i] == open {
                depth += 1;
            }
            i += 1;
        }
        Ok(None)
    }

    /// `%1`..`%9` — the subject must repeat what that capture held.
    fn match_capture(&mut self, s: usize, digit: u8) -> MatchResult {
        let index = (digit as isize) - (b'1' as isize);
        if index < 0 || index as usize >= self.capture.len() {
            return Err(format!("invalid capture index %{}", digit as char));
        }
        let (start, len) = self.capture[index as usize];
        if len == CAP_UNFINISHED {
            return Err(format!("invalid capture index %{}", digit as char));
        }
        let len = len.max(0) as usize;
        if self.src.len() - s >= len && self.src[start..start + len] == self.src[s..s + len] {
            return Ok(Some(s + len));
        }
        Ok(None)
    }

    /// Turn the recorded capture spans into values.
    fn collect(&self) -> Result<Vec<Capture>, String> {
        self.capture
            .iter()
            .map(|(start, len)| match *len {
                CAP_UNFINISHED => Err("unfinished capture".to_string()),
                CAP_POSITION => Ok(Capture::Position(start + 1)),
                n => Ok(Capture::Text(
                    self.src[*start..*start + n as usize].to_vec(),
                )),
            })
            .collect()
    }
}

/// Whether `c` is in the character class `cl` — `%a`, `%d`, and their negating uppercase forms.
fn match_class(c: u8, cl: u8) -> bool {
    let positive = match cl.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'g' => c.is_ascii_graphic(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        b's' => c == b' ' || (0x09..=0x0d).contains(&c),
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        // Not a class letter, so `%` was quoting a punctuation byte: `%.` is a literal dot.
        _ => return cl == c,
    };
    if cl.is_ascii_uppercase() {
        !positive
    } else {
        positive
    }
}

/// Find the first match of `pat` in `src` at or after `init`.
///
/// A leading `^` anchors the search to `init` rather than meaning "start of subject", which is
/// what makes `string.find(s, "^%s*", 5)` useful.
pub fn find(src: &[u8], pat: &[u8], init: usize) -> Result<Option<Match>, String> {
    let anchored = pat.first() == Some(&b'^');
    let pattern = if anchored { &pat[1..] } else { pat };
    let mut start = init.min(src.len());
    loop {
        let mut matcher = Matcher::new(src, pattern);
        if let Some(end) = matcher.do_match(start, 0)? {
            return Ok(Some(Match {
                start,
                end,
                captures: matcher.collect()?,
            }));
        }
        if anchored || start >= src.len() {
            return Ok(None);
        }
        start += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matched(src: &str, pat: &str) -> Option<String> {
        find(src.as_bytes(), pat.as_bytes(), 0)
            .unwrap()
            .map(|m| String::from_utf8_lossy(&src.as_bytes()[m.start..m.end]).into_owned())
    }

    fn captured(src: &str, pat: &str) -> Vec<Capture> {
        find(src.as_bytes(), pat.as_bytes(), 0)
            .unwrap()
            .map(|m| m.captures)
            .unwrap_or_default()
    }

    #[test]
    fn classes_and_quantifiers() {
        assert_eq!(matched("hello 42", "%d+").as_deref(), Some("42"));
        assert_eq!(matched("hello", "%a+").as_deref(), Some("hello"));
        // `-` is the lazy quantifier, so this stops at the first `>` rather than the last.
        assert_eq!(matched("<a><b>", "<.->").as_deref(), Some("<a>"));
        assert_eq!(matched("<a><b>", "<.*>").as_deref(), Some("<a><b>"));
        assert_eq!(matched("abc", "%d?").as_deref(), Some(""));
    }

    #[test]
    fn sets_ranges_and_negation() {
        assert_eq!(matched("x9y", "[0-9]").as_deref(), Some("9"));
        assert_eq!(matched("abc", "[^a]").as_deref(), Some("b"));
        // A `]` immediately after `[` is a member of the set, not its terminator.
        assert_eq!(matched("a]b", "[]]").as_deref(), Some("]"));
    }

    #[test]
    fn anchors_apply_where_the_search_starts() {
        assert!(matched("abc", "^abc").is_some());
        assert!(matched("xabc", "^abc").is_none());
        assert!(matched("abc", "c$").is_some());
        // A `$` that is not final is a literal.
        assert_eq!(matched("a$b", "a$b").as_deref(), Some("a$b"));
    }

    #[test]
    fn captures_record_text_and_position() {
        assert_eq!(
            captured("key=value", "(%w+)=(%w+)"),
            vec![
                Capture::Text(b"key".to_vec()),
                Capture::Text(b"value".to_vec())
            ]
        );
        assert_eq!(captured("abc", "a()b"), vec![Capture::Position(2)]);
    }

    #[test]
    fn backreferences_require_the_same_text_again() {
        assert!(matched("abcabc", "(abc)%1").is_some());
        assert!(matched("abcxyz", "(abc)%1").is_none());
    }

    #[test]
    fn balanced_match_counts_nesting() {
        assert_eq!(matched("f(a(b)c)d", "%b()").as_deref(), Some("(a(b)c)"));
        assert!(matched("f(a", "%b()").is_none());
    }

    #[test]
    fn frontier_fires_at_a_word_boundary() {
        assert_eq!(matched("  hello", "%f[%w]%w+").as_deref(), Some("hello"));
        // Also at offset zero, where the notional preceding byte is NUL.
        assert_eq!(matched("hi there", "%f[%w]%w+").as_deref(), Some("hi"));
    }

    /// Backtracking recurses once per *satisfied* quantifier, so a long enough chain of them would
    /// walk off the Rust stack. It has to come back as an error instead — a shell may not abort.
    #[test]
    fn a_pathological_pattern_errors_rather_than_overflowing() {
        let src = "a".repeat(400);
        let pat = "a?".repeat(400);
        let result = find(src.as_bytes(), pat.as_bytes(), 0);
        assert!(
            matches!(result, Err(ref e) if e.contains("too complex")),
            "{result:?}"
        );
    }

    #[test]
    fn malformed_patterns_are_reported() {
        assert!(find(b"x", b"[abc", 0).is_err());
        // Only once the matcher actually reaches it: Lua validates a pattern as it walks it, so a
        // malformed tail behind a part that cannot match is simply never seen.
        assert!(find(b"a", b"a%", 0).is_err());
        assert_eq!(matched("x", "abc%"), None);
        assert!(find(b"x", b"%f%w", 0).is_err());
    }
}
