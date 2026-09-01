//! When a verb name outranks an alias of the same name.
//!
//! The vocabulary is disjoint from POSIX and coreutils, not from names somebody has already taken:
//! `alias df=dfc` and `alias get="sudo sysget"` are ordinary, and they made `df | where …` and
//! `ls | get name` impossible, because an alias expands before the pipeline is planned.

/// Whether a later stage of this pipeline is itself a structured verb.
///
/// **A pipe is not enough, and assuming it was broke the most common alias there is.** `ls` is both
/// a verb and a real command, so `alias ls='ls -F'` with `ls | cat` has to keep meaning `ls -F`:
/// nothing in that line asks for rows. What marks the difference is another verb — `ls | get name`
/// and `df | where …` say plainly what they want, and `ls | wc -l` does not.
///
/// Looked ahead for rather than remembered, because a producer verb sits at the *head* of its
/// pipeline: when `df` is read, the `where` that gives it meaning has not been reached yet.
///
/// Quotes and everything `$(` opens are skipped, so a `|` inside them does not count — it belongs to
/// a command this one merely contains. `;`, `&`, `&&` and `||` all end the pipeline.
pub(super) fn a_verb_follows(chars: &[char], from: usize) -> bool {
    let mut depth = 0usize;
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '\'' | '"' => {
                let quote = chars[i];
                i += 1;
                while i < chars.len() && chars[i] != quote {
                    // A backslash escapes inside double quotes only; inside single ones it is text.
                    if quote == '"' && chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            '`' => {
                i += 1;
                while i < chars.len() && chars[i] != '`' {
                    i += 1;
                }
            }
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            // Inside a substitution the operators belong to somebody else's pipeline.
            _ if depth > 0 => {}
            '|' if chars.get(i + 1) != Some(&'|') => {
                let mut at = i + 1;
                while at < chars.len() && (chars[at] == ' ' || chars[at] == '\t') {
                    at += 1;
                }
                let start = at;
                while at < chars.len() && !" \t\n|;&<>".contains(chars[at]) {
                    at += 1;
                }
                let next: String = chars[start..at].iter().collect();
                if oslo_base::vocab::kind_of(&next) == Some("verb") {
                    return true;
                }
                i = at.saturating_sub(1);
            }
            ';' | '&' | '\n' => return false,
            _ => {}
        }
        i += 1;
    }
    false
}
