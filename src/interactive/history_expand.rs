//! History expansion: the `!` and `^` rewrites a shell performs on a line typed at a prompt.
//!
//! This lives in the binary, not in the library, and that is the whole point. History expansion
//! rewrites a line *before* it is parsed, so a `!` inside data would become a different command
//! than the one that was written. bash only does it for an interactive line and never for `-c`,
//! `sh script`, or a sourced file; keeping the code out of `rush::` means no library consumer and
//! no non-interactive path can reach it even by accident.
//!
//! What is supported, all against a `history` slice ordered oldest-first, where event number `n`
//! is `history[n - 1]` exactly as bash numbers the lines of a session:
//!
//! * events — `!!` (previous line), `!n`, `!-n`, `!str` (most recent line starting with `str`),
//!   `!?str?` (most recent line containing `str`);
//! * word designators — `!$`, `!^`, `!*` as shorthands on the previous line, and the general
//!   `!event:designator` with `0`, `n`, `^`, `$`, `*`, `n-m`, `n*`;
//! * quick substitution — `^old^new` and `^old^new^suffix`, only when `^` opens the line.
//!
//! Deliberately absent: the `:h`/`:t`/`:r`/`:s` modifiers and `!#`. They are rare, and an
//! unimplemented modifier is silently *wrong* rather than loudly missing, so the parser rejects
//! anything it does not know instead of guessing.

use std::fmt;

/// The result of running the expansion pass over one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expansion {
    /// The line held no history reference and must be run exactly as typed.
    ///
    /// Kept distinct from `Expanded` with an identical string because the caller echoes the
    /// rewritten line, and echoing a line the user just typed would be noise.
    Unchanged,
    /// The line was rewritten; this is what should be echoed, stored in history, and run.
    Expanded(String),
}

/// Why a line could not be expanded. A failing expansion runs nothing at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    /// The event reference names a line that is not in the history.
    EventNotFound(String),
    /// The event was found but the word designator does not select anything in it.
    BadWordSpecifier(String),
    /// `^old^new` where the previous line does not contain `old`.
    SubstitutionFailed(String),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventNotFound(event) => write!(f, "{event}: event not found"),
            Self::BadWordSpecifier(spec) => write!(f, "{spec}: bad word specifier"),
            Self::SubstitutionFailed(sub) => write!(f, "{sub}: substitution failed"),
        }
    }
}

impl std::error::Error for HistoryError {}

/// Rewrite one interactive line against `history` (oldest first).
///
/// # Errors
///
/// Returns [`HistoryError`] when a reference cannot be resolved. bash treats that as "nothing
/// ran": the line is discarded, `$?` is left alone, and the prompt comes back.
pub fn expand(line: &str, history: &[String]) -> Result<Expansion, HistoryError> {
    if let Some(result) = quick_substitution(line, history) {
        return result;
    }

    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut changed = false;

    while i < chars.len() {
        match chars[i] {
            // The backslash is *kept*, not consumed. History expansion only declines to act here;
            // the shell's own quote removal still has to see it, which is why bash prints `!` for
            // `echo \!` and `\!` for `echo "\!"`.
            '\\' if !in_single => {
                out.push('\\');
                i += 1;
                if i < chars.len() {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            '\'' if !in_double => {
                in_single = !in_single;
                out.push('\'');
                i += 1;
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push('"');
                i += 1;
            }
            // Single quotes are the only quoting that suppresses expansion; inside double quotes
            // bash still expands, so `echo "!$"` picks up the previous line's last word.
            '!' if !in_single && !is_glob_negation(&chars, i) => {
                match event_at(&chars, i, history)? {
                    Some((text, next)) => {
                        out.push_str(&text);
                        i = next;
                        changed = true;
                    }
                    None => {
                        out.push('!');
                        i += 1;
                    }
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }

    Ok(if changed {
        Expansion::Expanded(out)
    } else {
        Expansion::Unchanged
    })
}

/// Is the `!` at `at` the negation of a glob bracket expression rather than an event reference?
///
/// `ls [!a]*` has to keep working, so bash declines to expand a `!` that opens a bracket
/// expression which actually closes. The two escape hatches are bash's own: a bracket with no `]`
/// is not a bracket expression at all (`echo [!$` does expand), and `[!!]` expands because `!!` is
/// recognised before the bracket rule gets a say.
fn is_glob_negation(chars: &[char], at: usize) -> bool {
    at > 0
        && chars[at - 1] == '['
        && chars.get(at + 1) != Some(&'!')
        && chars[at + 1..].contains(&']')
}

/// `^old^new[^suffix]` — a `!!:s/old/new/` with no `!` in sight.
///
/// Returns `None` when the line is not a quick substitution at all, so the caller falls through to
/// the ordinary scan. Only a leading `^` counts: `echo a^b^c` is arithmetic-free data, not an edit.
fn quick_substitution(line: &str, history: &[String]) -> Option<Result<Expansion, HistoryError>> {
    let body = line.strip_prefix('^')?;
    let (old, rest) = match body.split_once('^') {
        Some(pair) => pair,
        // `^old` with no closing caret deletes `old`, which is what bash does with an empty
        // replacement.
        None => (body, ""),
    };
    let (new, suffix) = match rest.split_once('^') {
        Some((new, suffix)) => (new, suffix),
        None => (rest, ""),
    };

    let Some(previous) = history.last() else {
        return Some(Err(HistoryError::EventNotFound("!!".to_string())));
    };
    if old.is_empty() || !previous.contains(old) {
        return Some(Err(HistoryError::SubstitutionFailed(line.to_string())));
    }
    Some(Ok(Expansion::Expanded(format!(
        "{}{suffix}",
        previous.replacen(old, new, 1)
    ))))
}

/// Resolve the reference starting at the `!` in `chars[at]`.
///
/// `Ok(None)` means "this `!` is ordinary text" — bash leaves it alone before whitespace, `=` and
/// `(`, and at end of line, which is what keeps `a != b` and `echo hi!` working.
fn event_at(
    chars: &[char],
    at: usize,
    history: &[String],
) -> Result<Option<(String, usize)>, HistoryError> {
    let i = at + 1;
    let Some(&next) = chars.get(i) else {
        return Ok(None);
    };
    if matches!(next, ' ' | '\t' | '\n' | '=' | '(') {
        return Ok(None);
    }

    // `!$`, `!^`, `!*` are word designators on the previous line with the event left implicit.
    if matches!(next, '$' | '^' | '*') {
        let entry = nth_back(history, 1)?;
        let (text, end) = word_designator(chars, i, &entry)?;
        return Ok(Some((text, end)));
    }

    let (entry, mut i) = if next == '!' {
        (nth_back(history, 1)?, i + 1)
    } else if next == '-' {
        let (digits, end) = take_digits(chars, i + 1);
        if digits.is_empty() {
            return Ok(None);
        }
        let back: usize = digits
            .parse()
            .map_err(|_| HistoryError::EventNotFound(format!("!-{digits}")))?;
        (nth_back(history, back)?, end)
    } else if next.is_ascii_digit() {
        let (digits, end) = take_digits(chars, i);
        let number: usize = digits
            .parse()
            .map_err(|_| HistoryError::EventNotFound(format!("!{digits}")))?;
        // Event numbers are 1-based and count from the oldest line the shell knows about.
        let entry = history
            .get(number.wrapping_sub(1))
            .filter(|_| number > 0)
            .ok_or_else(|| HistoryError::EventNotFound(format!("!{digits}")))?;
        (entry.clone(), end)
    } else if next == '?' {
        let (needle, end) = take_until(chars, i + 1, |c| c == '?');
        // The closing `?` is optional at end of line.
        let end = if chars.get(end) == Some(&'?') {
            end + 1
        } else {
            end
        };
        let entry = history
            .iter()
            .rev()
            .find(|e| e.contains(&needle))
            .ok_or_else(|| HistoryError::EventNotFound(format!("!?{needle}")))?;
        (entry.clone(), end)
    } else {
        let (prefix, end) = take_until(chars, i, is_event_delimiter);
        if prefix.is_empty() {
            return Ok(None);
        }
        let entry = history
            .iter()
            .rev()
            .find(|e| e.starts_with(&prefix))
            .ok_or_else(|| HistoryError::EventNotFound(format!("!{prefix}")))?;
        (entry.clone(), end)
    };

    // A word designator may follow, with the `:` optional when it starts with `^`, `$` or `*`.
    match chars.get(i) {
        Some(':') => {
            i += 1;
            let (text, end) = word_designator(chars, i, &entry)?;
            Ok(Some((text, end)))
        }
        Some('^' | '$' | '*') => {
            let (text, end) = word_designator(chars, i, &entry)?;
            Ok(Some((text, end)))
        }
        _ => Ok(Some((entry, i))),
    }
}

/// The `n` in `!-n` / the `!!` shorthand: `back == 1` is the previous line.
fn nth_back(history: &[String], back: usize) -> Result<String, HistoryError> {
    history
        .len()
        .checked_sub(back)
        .filter(|_| back > 0)
        .and_then(|idx| history.get(idx))
        .cloned()
        .ok_or_else(|| {
            HistoryError::EventNotFound(if back == 1 {
                "!!".to_string()
            } else {
                format!("!-{back}")
            })
        })
}

/// Apply the designator starting at `chars[at]` to `entry`, returning the text and the index just
/// past the designator.
///
/// Words are split on whitespace. bash's history library tokenises with quoting rules, so a
/// designator that lands inside a quoted argument can differ — a limitation worth having over a
/// second, subtly different word splitter.
fn word_designator(
    chars: &[char],
    at: usize,
    entry: &str,
) -> Result<(String, usize), HistoryError> {
    let words: Vec<&str> = entry.split_whitespace().collect();
    let last = words.len().saturating_sub(1);
    let Some(&c) = chars.get(at) else {
        return Err(HistoryError::BadWordSpecifier(":".to_string()));
    };

    let pick = |from: usize, to: usize, spec: &str| -> Result<String, HistoryError> {
        if from > to || to >= words.len() {
            return Err(HistoryError::BadWordSpecifier(format!(":{spec}")));
        }
        Ok(words[from..=to].join(" "))
    };

    match c {
        // `^` is the *first argument*, not the command word: `!^` of `echo a b` is `a`.
        '^' => Ok((pick(1, 1, "^")?, at + 1)),
        '$' => Ok((pick(last, last, "$")?, at + 1)),
        // `!*` on a line with no arguments is empty, not an error — bash agrees.
        '*' => Ok((
            if words.len() > 1 {
                words[1..].join(" ")
            } else {
                String::new()
            },
            at + 1,
        )),
        d if d.is_ascii_digit() => {
            let (digits, end) = take_digits(chars, at);
            let from: usize = digits
                .parse()
                .map_err(|_| HistoryError::BadWordSpecifier(format!(":{digits}")))?;
            match chars.get(end) {
                Some('*') => Ok((pick(from, last, &format!("{digits}*"))?, end + 1)),
                Some('-') => {
                    let (to_digits, to_end) = take_digits(chars, end + 1);
                    if to_digits.is_empty() {
                        // `n-` is "n through the second to last word".
                        let to = last.saturating_sub(1);
                        return Ok((pick(from, to, &format!("{digits}-"))?, end + 1));
                    }
                    let to: usize = to_digits
                        .parse()
                        .map_err(|_| HistoryError::BadWordSpecifier(format!(":{digits}-")))?;
                    Ok((pick(from, to, &format!("{digits}-{to_digits}"))?, to_end))
                }
                _ => Ok((pick(from, from, &digits)?, end)),
            }
        }
        other => Err(HistoryError::BadWordSpecifier(format!(":{other}"))),
    }
}

fn take_digits(chars: &[char], at: usize) -> (String, usize) {
    take_until(chars, at, |c| !c.is_ascii_digit())
}

fn take_until(chars: &[char], at: usize, stop: impl Fn(char) -> bool) -> (String, usize) {
    let mut end = at;
    while end < chars.len() && !stop(chars[end]) {
        end += 1;
    }
    (chars[at..end].iter().collect(), end)
}

/// Where a `!str` event reference ends.
///
/// Everything that could start a new shell token, plus the characters that introduce a word
/// designator, so `!echo:$` and `!echo;ls` both split where a reader expects.
fn is_event_delimiter(c: char) -> bool {
    c.is_whitespace() || "!\"'();<>|&:^$*=?`\\".contains(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> Vec<String> {
        ["echo alpha beta gamma", "ls -l /tmp", "echo A B"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn expanded(line: &str) -> String {
        match expand(line, &history()) {
            Ok(Expansion::Expanded(s)) => s,
            other => panic!("{line:?} expected an expansion, got {other:?}"),
        }
    }

    fn err(line: &str) -> HistoryError {
        match expand(line, &history()) {
            Err(e) => e,
            other => panic!("{line:?} expected an error, got {other:?}"),
        }
    }

    #[test]
    fn events_and_designators_match_bash() {
        // (input, expansion) — every row checked against `bash -i` with the same history.
        let cases = [
            ("!!", "echo A B"),
            ("!-1", "echo A B"),
            ("!-3", "echo alpha beta gamma"),
            ("!1", "echo alpha beta gamma"),
            ("!3", "echo A B"),
            ("!ls", "ls -l /tmp"),
            ("!echo", "echo A B"),
            ("!?tmp?", "ls -l /tmp"),
            ("!?tmp", "ls -l /tmp"),
            ("sudo !!", "sudo echo A B"),
            ("echo pre !$ post", "echo pre B post"),
            ("echo pre !^ post", "echo pre A post"),
            ("echo pre !* post", "echo pre A B post"),
            ("echo <!!:0>", "echo <echo>"),
            ("echo <!1:2>", "echo <beta>"),
            ("echo <!1:1-2>", "echo <alpha beta>"),
            ("echo <!1:1*>", "echo <alpha beta gamma>"),
            ("echo <!1:1->", "echo <alpha beta>"),
            ("echo <!1:$>", "echo <gamma>"),
            // `[!!]` is the one bracket bash still expands, because `!!` wins over the glob rule.
            ("echo [!!]", "echo [echo A B]"),
            // A `[` with no `]` is not a bracket expression, so the reference stands.
            ("echo [!$", "echo [B"),
            ("echo [a!$]", "echo [aB]"),
            // Expansion happens inside double quotes but not inside single quotes.
            ("echo \"!$\"", "echo \"B\""),
            ("echo 'x' !$", "echo 'x' B"),
            // Two references in one line. Both resolve against the stored history, so the second
            // `!!` is still the previous *entry* and not what the first one just produced.
            ("!ls | !!", "ls -l /tmp | echo A B"),
            ("echo X!^X", "echo XAX"),
        ];
        for (input, want) in cases {
            assert_eq!(expanded(input), want, "expanding {input:?}");
        }
    }

    #[test]
    fn lines_without_a_reference_are_left_alone() {
        // A `!` that bash treats as data: before whitespace, `=` and `(`, at end of line, inside
        // single quotes, and after a backslash.
        let cases = [
            "echo hello",
            "echo a! b",
            "echo a!=b",
            "test 1 != 2",
            "echo 5!",
            "echo !",
            "echo '!!'",
            "echo '!$'",
            "echo \\!",
            "echo \"\\!\"",
            "! true",
            "if ! grep -q x f; then echo no; fi",
            "echo a!(b)",
            // Glob bracket negation, the reason the `[!` rule exists at all.
            "ls [!a]*",
            "echo [!$]",
            "echo [!1]",
        ];
        for input in cases {
            assert_eq!(
                expand(input, &history()),
                Ok(Expansion::Unchanged),
                "{input:?} must be left alone"
            );
        }
    }

    #[test]
    fn a_backslash_survives_into_the_shells_own_quote_removal() {
        // The pass must not eat the backslash: `echo \!` prints `!` only because the shell still
        // sees the escape afterwards.
        assert_eq!(expand("echo \\!", &history()), Ok(Expansion::Unchanged));
        assert_eq!(expanded("echo \\a !!"), "echo \\a echo A B");
    }

    #[test]
    fn quick_substitution_edits_the_previous_line() {
        assert_eq!(expanded("^A^Z"), "echo Z B");
        assert_eq!(expanded("^A^Z^"), "echo Z B");
        assert_eq!(expanded("^A^Z^ | cat"), "echo Z B | cat");
        // Only the first occurrence, and only when `^` opens the line.
        assert_eq!(expand("echo a^b^c", &history()), Ok(Expansion::Unchanged));
        assert_eq!(
            err("^nope^x"),
            HistoryError::SubstitutionFailed("^nope^x".to_string())
        );
    }

    #[test]
    fn unresolvable_references_are_errors_not_guesses() {
        assert_eq!(
            err("!nosuch"),
            HistoryError::EventNotFound("!nosuch".to_string())
        );
        assert_eq!(err("!99"), HistoryError::EventNotFound("!99".to_string()));
        assert_eq!(err("!0"), HistoryError::EventNotFound("!0".to_string()));
        assert_eq!(err("!-9"), HistoryError::EventNotFound("!-9".to_string()));
        assert_eq!(
            err("!?zzz?"),
            HistoryError::EventNotFound("!?zzz".to_string())
        );
        // Out of range and unknown designators both refuse rather than silently drop the word.
        assert_eq!(
            err("echo !!:9"),
            HistoryError::BadWordSpecifier(":9".to_string())
        );
        assert_eq!(
            err("echo !!:h"),
            HistoryError::BadWordSpecifier(":h".to_string())
        );
    }

    #[test]
    fn an_empty_history_cannot_satisfy_any_reference() {
        assert_eq!(
            expand("!!", &[]),
            Err(HistoryError::EventNotFound("!!".to_string()))
        );
        assert_eq!(
            expand("^a^b", &[]),
            Err(HistoryError::EventNotFound("!!".to_string()))
        );
        assert_eq!(expand("echo hi", &[]), Ok(Expansion::Unchanged));
    }

    #[test]
    fn error_text_names_the_reference_that_failed() {
        assert_eq!(
            HistoryError::EventNotFound("!x".into()).to_string(),
            "!x: event not found"
        );
        assert_eq!(
            HistoryError::BadWordSpecifier(":9".into()).to_string(),
            ":9: bad word specifier"
        );
        assert_eq!(
            HistoryError::SubstitutionFailed("^a^b".into()).to_string(),
            "^a^b: substitution failed"
        );
    }
}
