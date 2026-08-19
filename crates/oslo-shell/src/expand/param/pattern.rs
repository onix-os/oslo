//! Anchored shell-pattern matching for the `${v#pat}` / `${v%pat}` / `${v/pat/rep}` operators.
//!
//! The operators these back are *pattern* operators, not substring searches. `${p##*/}` is
//! basename and `${f%.*}` drops an extension only because `*` is a metacharacter and the match is
//! anchored to one end of the value. An earlier implementation used `str::find`/`rfind` on the raw
//! pattern text, which made both idioms silently return the wrong string — the failure mode this
//! module exists to prevent.
//!
//! Every entry point works in *characters*, never bytes, so a multi-byte value cannot be cut
//! mid-character.

use crate::expand::glob::ShellPattern;

/// Every index at which `s` may be cut, ascending: each character boundary plus the end.
fn cuts(s: &str) -> Vec<usize> {
    s.char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(s.len()))
        .collect()
}

/// `${v#pat}` (`longest == false`) and `${v##pat}`.
///
/// Tests every prefix of `value`; the shortest form takes the first one that matches, the longest
/// form the last. The empty prefix is a candidate, so `${v#}` and a non-matching pattern both
/// leave the value alone.
pub fn remove_prefix(value: &str, pattern: &ShellPattern, longest: bool) -> String {
    let mut candidates = cuts(value);
    if longest {
        candidates.reverse();
    }
    for cut in candidates {
        if pattern.matches(&value[..cut]) {
            return value[cut..].to_string();
        }
    }
    value.to_string()
}

/// `${v%pat}` (`longest == false`) and `${v%%pat}`.
///
/// The mirror image of [`remove_prefix`]: a *short* suffix starts at a *high* index, so the
/// shortest form walks the cut points from the end. Getting this direction backwards is what made
/// `v=abcabc; ${v%abc}` yield `""` instead of `abc`.
pub fn remove_suffix(value: &str, pattern: &ShellPattern, longest: bool) -> String {
    let mut candidates = cuts(value);
    if !longest {
        candidates.reverse();
    }
    for cut in candidates {
        if pattern.matches(&value[cut..]) {
            return value[..cut].to_string();
        }
    }
    value.to_string()
}

/// The longest match of `pattern` starting exactly at byte index `start`, as an end index.
fn match_at(value: &str, start: usize, pattern: &ShellPattern) -> Option<usize> {
    let rest = &value[start..];
    let mut ends = cuts(rest);
    ends.reverse();
    ends.into_iter()
        .find(|&end| pattern.matches(&rest[..end]))
        .map(|end| start + end)
}

/// `${v/pat/rep}` — replace the leftmost match, or every match when `all`.
///
/// Matching is longest-at-each-position, left to right. A zero-length match is ignored rather
/// than replaced — `v=ab; ${v//x*/-}` is `ab` in bash, not `-a-b-` — which also keeps the scan
/// from standing still.
pub fn replace(value: &str, pattern: &ShellPattern, replacement: &str, all: bool) -> String {
    let mut out = String::new();
    let mut pos = 0;
    let mut replaced = false;
    while pos < value.len() {
        let hit = if all || !replaced {
            match_at(value, pos, pattern).filter(|&end| end > pos)
        } else {
            None
        };
        if let Some(end) = hit {
            out.push_str(replacement);
            pos = end;
            replaced = true;
            continue;
        }
        // No usable match here: carry one character across and move on.
        let ch = value[pos..].chars().next().expect("pos is a char boundary");
        out.push(ch);
        pos += ch.len_utf8();
    }
    out
}

/// `${v/#pat/rep}` — replace only a match anchored at the start, longest first.
pub fn replace_prefix(value: &str, pattern: &ShellPattern, replacement: &str) -> String {
    match match_at(value, 0, pattern) {
        Some(end) => format!("{replacement}{}", &value[end..]),
        None => value.to_string(),
    }
}

/// `${v/%pat/rep}` — replace only a match anchored at the end, longest first.
pub fn replace_suffix(value: &str, pattern: &ShellPattern, replacement: &str) -> String {
    for cut in cuts(value) {
        if pattern.matches(&value[cut..]) {
            return format!("{}{replacement}", &value[..cut]);
        }
    }
    value.to_string()
}

/// `${v:offset:length}` over characters, with bash's negative-index rules.
///
/// A negative offset counts back from the end; a negative length names an end position rather
/// than a count. `Err` carries the offending length for the caller to report, because bash makes
/// `${v:2:-9}` a fatal expansion error rather than an empty string.
pub fn substring(value: &str, offset: i64, length: Option<i64>) -> Result<String, i64> {
    let chars: Vec<char> = value.chars().collect();
    let len = chars.len() as i64;

    let start = if offset < 0 {
        // Still negative after counting back from the end means the window starts before the
        // string and bash yields nothing at all.
        let from_end = len + offset;
        if from_end < 0 {
            return Ok(String::new());
        }
        from_end
    } else {
        offset.min(len)
    };

    let end = match length {
        None => len,
        Some(n) if n < 0 => {
            let e = len + n;
            if e < start {
                return Err(n);
            }
            e
        }
        Some(n) => (start + n).min(len),
    };

    Ok(chars[start as usize..end.max(start) as usize]
        .iter()
        .collect())
}

/// `${v^pat}` / `${v,,pat}` — convert the characters matching `pattern` (all of them when it is
/// `None`), either just the first or every one.
pub fn convert_case(value: &str, pattern: Option<&ShellPattern>, upper: bool, all: bool) -> String {
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        let selected = match pattern {
            None => true,
            Some(p) => p.matches(&ch.to_string()),
        };
        // The single-character forms examine the first character and no other, so
        // `v=hello; ${v^l}` leaves the value alone: `h` is not what the pattern named.
        let eligible = selected && (all || index == 0);
        if eligible && upper {
            out.extend(ch.to_uppercase());
        } else if eligible {
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests spell patterns as source text, so every metacharacter in them is meant as one.
    fn pat(text: &str) -> ShellPattern {
        ShellPattern::from_unquoted(text)
    }

    #[test]
    fn prefix_removal_is_anchored_and_globs() {
        let p = "/usr/local/lib/libfoo.so";
        assert_eq!(remove_prefix(p, &pat("*/"), true), "libfoo.so");
        assert_eq!(
            remove_prefix(p, &pat("*/"), false),
            "usr/local/lib/libfoo.so"
        );
        assert_eq!(
            remove_prefix(p, &pat("/usr"), false),
            "/local/lib/libfoo.so"
        );
        assert_eq!(remove_prefix(p, &pat("*."), true), "so");
        // Unanchored substring search would have stripped nothing here.
        assert_eq!(remove_prefix("abcabc", &pat("abc"), false), "abc");
        assert_eq!(remove_prefix("abcabc", &pat("abc"), true), "abc");
        assert_eq!(remove_prefix("abc", &pat("x"), true), "abc");
        assert_eq!(remove_prefix("abc", &pat(""), false), "abc");
    }

    #[test]
    fn suffix_removal_picks_the_right_length() {
        assert_eq!(
            remove_suffix("archive.tar.gz", &pat(".*"), false),
            "archive.tar"
        );
        assert_eq!(remove_suffix("archive.tar.gz", &pat(".*"), true), "archive");
        let p = "/usr/local/lib/libfoo.so";
        assert_eq!(remove_suffix(p, &pat("/*"), false), "/usr/local/lib");
        assert_eq!(remove_suffix(p, &pat("/*"), true), "");
        // The regression that started this: shortest-suffix must leave the first `abc` standing.
        assert_eq!(remove_suffix("abcabc", &pat("abc"), false), "abc");
        assert_eq!(remove_suffix("abcabc", &pat("abc"), true), "abc");
        assert_eq!(remove_suffix("abc", &pat("x"), false), "abc");
    }

    /// The quoting a pattern arrived with is what the operator matches by: `${v#"a*"}` strips a
    /// literal `a*` and leaves `axc` alone.
    #[test]
    fn a_quoted_metacharacter_is_a_character() {
        use crate::expand::word::{Origin, Run};
        let literal = crate::expand::glob::pattern_from_runs(&[Run::new("a*", Origin::Quoted)]);
        assert_eq!(remove_prefix("a*c", &literal, false), "c");
        assert_eq!(remove_prefix("axc", &literal, false), "axc");
    }

    #[test]
    fn replacement_honours_scope() {
        assert_eq!(replace("a-b-c", &pat("-"), "+", false), "a+b-c");
        assert_eq!(replace("a-b-c", &pat("-"), "+", true), "a+b+c");
        assert_eq!(
            replace("one.two.three", &pat("."), " ", true),
            "one two three"
        );
        assert_eq!(replace_prefix("a-b-c", &pat("a"), "A"), "A-b-c");
        assert_eq!(replace_prefix("a-b-c", &pat("b"), "B"), "a-b-c");
        assert_eq!(replace_suffix("a-b-c", &pat("c"), "C"), "a-b-C");
        assert_eq!(replace_suffix("a-b-c", &pat("b"), "B"), "a-b-c");
    }

    /// A pattern that matches only the empty string replaces nothing, and does not spin.
    #[test]
    fn an_empty_match_is_ignored() {
        assert_eq!(replace("ab", &pat("x*"), "-", true), "ab");
        assert_eq!(replace("ab", &pat("x*"), "-", false), "ab");
        assert_eq!(replace("ab", &pat(""), "-", true), "ab");
        // The anchored forms do accept one, which is how `${v/#/X}` prepends.
        assert_eq!(replace_prefix("ab", &pat(""), "X"), "Xab");
        assert_eq!(replace_suffix("ab", &pat(""), "X"), "abX");
    }

    /// Longest-at-each-position, not shortest: `*` eats the rest of the value in one match.
    #[test]
    fn replacement_takes_the_longest_match_at_each_position() {
        assert_eq!(replace("aaa", &pat("a*"), "X", true), "X");
        assert_eq!(replace("abc", &pat("?"), "X", true), "XXX");
    }

    #[test]
    fn substring_handles_negative_indices() {
        let v = "abcdefgh";
        assert_eq!(substring(v, 2, Some(3)), Ok("cde".into()));
        assert_eq!(substring(v, 2, None), Ok("cdefgh".into()));
        assert_eq!(substring(v, 0, Some(1)), Ok("a".into()));
        assert_eq!(substring(v, -3, None), Ok("fgh".into()));
        assert_eq!(substring(v, -3, Some(2)), Ok("fg".into()));
        assert_eq!(substring(v, -20, None), Ok(String::new()));
        assert_eq!(substring(v, 20, None), Ok(String::new()));
        assert_eq!(substring(v, 2, Some(-3)), Ok("cde".into()));
        assert_eq!(substring(v, 2, Some(0)), Ok(String::new()));
        assert_eq!(substring(v, 2, Some(-9)), Err(-9));
    }

    /// Characters, not bytes: cutting a multi-byte value by byte index would panic.
    #[test]
    fn substring_counts_characters() {
        assert_eq!(substring("héllo", 1, Some(2)), Ok("él".into()));
        assert_eq!(remove_prefix("héllo", &pat("h?"), false), "llo");
    }

    #[test]
    fn case_conversion_respects_the_doubled_form() {
        assert_eq!(convert_case("hello", None, true, true), "HELLO");
        assert_eq!(convert_case("hello", None, true, false), "Hello");
        assert_eq!(convert_case("WORLD", None, false, true), "world");
        assert_eq!(convert_case("WORLD", None, false, false), "wORLD");
        assert_eq!(convert_case("hello", Some(&pat("l")), true, true), "heLLo");
        // `${v^l}`: only the first character is a candidate, and it is not an `l`.
        assert_eq!(convert_case("hello", Some(&pat("l")), true, false), "hello");
        assert_eq!(
            convert_case("hello", Some(&pat("[el]")), true, true),
            "hELLo"
        );
    }
}
