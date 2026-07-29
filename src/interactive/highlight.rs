//! Syntax colouring for the line being typed.
//!
//! The colouring itself is unchanged; what moved is how "is this a real command" is answered.
//! It used to be `which::which` per token on every refresh — a `$PATH` walk per keystroke on top
//! of the hinter's. The cached command index knows the same thing without touching the disk, and
//! only a name containing `/` (a path, not a lookup) still needs `which`.

use super::command_index::CommandIndex;

#[derive(Debug, PartialEq, Eq)]
pub enum TokenType {
    Command,
    Flag,
    String,
    Variable,
    Operator,
    Plain,
}

/// Split a line into coloured spans. The concatenation of the spans is the original line.
pub fn tokenize_for_highlight(line: &str) -> Vec<(String, TokenType)> {
    let mut result = Vec::new();
    let mut chars = line.chars().peekable();
    let mut is_first_word = true;

    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            let mut space_str = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    space_str.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            result.push((space_str, TokenType::Plain));
            continue;
        }

        if matches!(ch, '|' | '&' | ';' | '<' | '>') {
            let mut op_str = String::new();
            op_str.push(ch);
            chars.next();
            if let Some(&next_ch) = chars.peek()
                && ((ch == '|' && next_ch == '|')
                    || (ch == '&' && next_ch == '&')
                    || (ch == '>' && next_ch == '>'))
            {
                op_str.push(next_ch);
                chars.next();
            }
            result.push((op_str, TokenType::Operator));
            is_first_word = true;
            continue;
        }

        if ch == '\'' || ch == '"' {
            let quote = ch;
            let mut str_lit = String::new();
            str_lit.push(quote);
            chars.next();
            while let Some(&c) = chars.peek() {
                str_lit.push(c);
                chars.next();
                if c == quote {
                    break;
                }
            }
            result.push((str_lit, TokenType::String));
            is_first_word = false;
            continue;
        }

        if ch == '$' {
            let mut var_str = String::new();
            var_str.push(ch);
            chars.next();
            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '_' || c == '?' || c == '!' || c == '#' {
                    var_str.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            result.push((var_str, TokenType::Variable));
            is_first_word = false;
            continue;
        }

        let mut word_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || matches!(c, '|' | '&' | ';' | '<' | '>' | '\'' | '"' | '$') {
                break;
            }
            word_str.push(c);
            chars.next();
        }

        if is_first_word {
            result.push((word_str, TokenType::Command));
            is_first_word = false;
        } else if word_str.starts_with('-') {
            result.push((word_str, TokenType::Flag));
        } else {
            result.push((word_str, TokenType::Plain));
        }
    }

    result
}

/// Whether a command name in the line resolves to something runnable.
///
/// `path` is the shell's `$PATH`; `known` answers for builtins, aliases and functions, which the
/// index does not track because they change without any file changing.
pub fn command_resolves(name: &str, path: &str, known: impl FnOnce(&str) -> bool) -> bool {
    if name.is_empty() {
        return false;
    }
    if known(name) {
        return true;
    }
    if name.contains('/') {
        // A path, not a lookup: `./configure`, `/usr/bin/env`. Rare enough on a prompt line that
        // the stat is not worth caching.
        return which::which(name).is_ok();
    }
    CommandIndex::contains(path, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_reassemble_into_the_original_line() {
        for line in [
            "echo hi",
            "ls -l | wc -l",
            "echo \"a b\" $HOME",
            "git commit -m 'x' && true",
        ] {
            let joined: String = tokenize_for_highlight(line)
                .iter()
                .map(|(s, _)| s.as_str())
                .collect();
            assert_eq!(joined, line);
        }
    }

    #[test]
    fn the_first_word_of_each_command_is_a_command() {
        let toks = tokenize_for_highlight("ls | wc");
        let commands: Vec<&str> = toks
            .iter()
            .filter(|(_, t)| *t == TokenType::Command)
            .map(|(s, _)| s.as_str())
            .collect();
        assert_eq!(commands, vec!["ls", "wc"]);
    }

    #[test]
    fn a_builtin_resolves_without_touching_the_disk() {
        assert!(command_resolves("cd", "/nonexistent", |n| n == "cd"));
        assert!(!command_resolves(
            "definitely-not-a-command",
            "/nonexistent",
            |_| false
        ));
    }
}
