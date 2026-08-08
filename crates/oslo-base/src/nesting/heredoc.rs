//! Recognising here-document bodies, so the nesting scan can skip them.
//!
//! Split from `nesting.rs` because it answers one question — where does a body start and end —
//! and it is the question the scan got wrong twice: once by not asking it at all (a C program in
//! `config.guess`'s `<<EOF` counted as 22 unmatched openers), and once by missing the
//! backslash-quoted spelling `<<\_ACEOF`, which autoconf uses 15 times per `configure` and whose
//! bodies are English prose full of the words "do", "if" and "case".

/// Remove here-document bodies, which are data and not shell.
///
/// The scan counts openers, and a heredoc body is free to contain whatever it likes: `config.guess`
/// — the autoconf script in every autotools project — writes a **C program** through `<<EOF`, and
/// its braces were counted as shell nesting. That is 22 phantom unmatched openers, and it is why
/// this guard rejected a script every distro builds against.
///
/// Deliberately approximate in the safe direction: the delimiter is recognised by shape
/// (`<<WORD`, `<<-WORD`, `<<'WORD'`) rather than by re-lexing, and anything not recognised is left
/// in place to be counted as before. Requiring an identifier after `<<` is also what keeps the
/// arithmetic shift in `$(( 1 << 2 ))` from being read as a heredoc.
pub(super) fn strip_heredoc_bodies(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut lines = input.lines();
    while let Some(line) = lines.next() {
        out.push_str(line);
        out.push('\n');
        for (delimiter, strip_tabs) in heredoc_delimiters(line) {
            for body in lines.by_ref() {
                let candidate = if strip_tabs {
                    body.trim_start_matches('\t')
                } else {
                    body
                };
                if candidate == delimiter {
                    break;
                }
            }
        }
    }
    out
}

/// The delimiters a line opens here-documents for, in the order they will be read.
pub fn heredoc_delimiters(line: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] != '<' || chars[i + 1] != '<' {
            i += 1;
            continue;
        }
        // `<<<` is a here-string: its operand is a word on the same line, not a body.
        if chars.get(i + 2) == Some(&'<') {
            i += 3;
            continue;
        }
        let mut j = i + 2;
        let strip_tabs = chars.get(j) == Some(&'-');
        if strip_tabs {
            j += 1;
        }
        while chars.get(j).is_some_and(|c| *c == ' ') {
            j += 1;
        }
        // The delimiter may be quoted to stop the body being expanded, and a *backslash* is the
        // third way to spell that — `<<\_ACEOF`, which autoconf uses 15 times in a single
        // `configure`. Missing it left those bodies to be scanned as shell, and `configure`'s
        // own --help text contains the words "do", "if" and "case" in prose, each of which
        // opened a construct that never closed.
        if chars.get(j) == Some(&'\\') {
            j += 1;
        }
        let quote = match chars.get(j) {
            Some('\'') => Some('\''),
            Some('"') => Some('"'),
            _ => None,
        };
        if quote.is_some() {
            j += 1;
        }
        let start = j;
        // A delimiter is a word, so it cannot start with a digit: `$(( 1 << 2 ))` is an
        // arithmetic shift, and reading `2` as a heredoc name would swallow the rest of the file.
        if !chars
            .get(j)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_')
        {
            i += 2;
            continue;
        }
        while chars
            .get(j)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
        {
            j += 1;
        }
        if j == start {
            i += 2;
            continue;
        }
        found.push((chars[start..j].iter().collect::<String>(), strip_tabs));
        i = j;
    }
    found
}
