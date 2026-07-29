//! Scanning `$` expansions.
//!
//! Covers `$var`, `${...}` in all its parameter-expansion forms, `$(...)` and backtick command
//! substitution, and `$((...))` arithmetic.

use super::ansi_c;
use super::quoting::parse_word_source;
use super::scanner::{Lexer, is_special_param, is_var_name_char};
use crate::ast::{ParamExpansion, WordPart};
use crate::error::{Result, ShellError};

impl Lexer<'_> {
    /// Scan whatever follows a `$`, the cursor sitting on the character after it.
    ///
    /// `in_double_quotes` is not cosmetic: `$'…'` and `$"…"` are only special *outside* quotes.
    /// bash prints `$'a\tb'` for `echo "$'a\tb'"`, so inside a double-quoted run the `$` has to
    /// stay the literal it always was.
    pub(super) fn scan_dollar_expansion(&mut self, in_double_quotes: bool) -> Result<WordPart> {
        match self.current_char() {
            Some('{') => {
                self.advance();
                self.scan_braced_param()
            }
            Some('(') => {
                self.advance();
                if self.current_char() == Some('(') {
                    self.advance();
                    // `$((` is ambiguous: arithmetic, or a substitution whose first command is a
                    // subshell. Scan one balanced paren group and let the character after it
                    // decide — `$((1+2))` closes twice in a row, `$((cd /) ; pwd)` does not.
                    let inner = self.scan_raw_delimited('(', ')', "arithmetic expansion", 0)?;
                    if self.current_char() == Some(')') {
                        self.advance();
                        return Ok(WordPart::Arithmetic(inner));
                    }
                    let rest = self.scan_raw_delimited('(', ')', "command substitution", 0)?;
                    return Ok(WordPart::CommandSubstitution(format!("({inner}){rest}")));
                }
                let cmd = self.scan_raw_delimited('(', ')', "command substitution", 0)?;
                Ok(WordPart::CommandSubstitution(cmd))
            }
            Some('\'') if !in_double_quotes => {
                self.advance();
                let decoded = self.scan_ansi_c_quote()?;
                // Single-quoted, not literal: the decoded text is data. A decoded `*` must not
                // glob and a decoded newline must not field-split.
                Ok(WordPart::SingleQuoted(decoded))
            }
            Some('"') if !in_double_quotes => {
                self.advance();
                // `$"…"` marks a translatable string. With no message catalogue there is nothing
                // to translate, so it is exactly a double-quoted run — which is also what bash
                // does when the string is not in the catalogue.
                Ok(WordPart::DoubleQuoted(self.scan_double_quote()?))
            }
            Some(ch) if is_var_name_char(ch) || is_special_param(ch) => {
                let mut name = String::new();
                if is_special_param(ch) {
                    name.push(ch);
                    self.advance();
                } else {
                    while let Some(c) = self.current_char() {
                        if is_var_name_char(c) {
                            name.push(c);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                Ok(WordPart::Variable {
                    name,
                    expansion_type: ParamExpansion::Normal,
                })
            }
            _ => Ok(WordPart::Literal("$".to_string())),
        }
    }

    /// Scan a `` `cmd` `` substitution, the cursor on the opening backquote.
    ///
    /// Shared by [`Lexer::scan_word`] and [`Lexer::scan_double_quote`], because backquotes are
    /// live in both. The body is *not* the raw source: inside backquotes only `` \` ``, `\\` and
    /// `\$` are escapes, and the shell strips those backslashes before parsing what is left.
    /// Every other backslash is data, which is why this cannot reuse the raw `$( )` scanner.
    pub(super) fn scan_backquote_substitution(&mut self) -> Result<WordPart> {
        self.advance();
        let mut cmd = String::new();

        while let Some(ch) = self.current_char() {
            match ch {
                '`' => {
                    self.advance();
                    return Ok(WordPart::CommandSubstitution(cmd));
                }
                '\\' if matches!(self.peek_char(), Some('`') | Some('\\') | Some('$')) => {
                    self.advance();
                    if let Some(escaped) = self.advance() {
                        cmd.push(escaped);
                    }
                }
                _ => {
                    cmd.push(ch);
                    self.advance();
                }
            }
        }

        Err(ShellError::SyntaxError(
            "Unterminated backquote".to_string(),
        ))
    }

    /// Scan the body of `$'…'`, the cursor just past the opening quote.
    ///
    /// Only `\'` matters while scanning: it is the one escape that stops the string from ending.
    /// Everything else is left for [`ansi_c::decode`].
    fn scan_ansi_c_quote(&mut self) -> Result<String> {
        let mut raw = String::new();

        while let Some(ch) = self.current_char() {
            match ch {
                '\'' => {
                    self.advance();
                    return Ok(ansi_c::decode(&raw));
                }
                '\\' => {
                    self.advance();
                    raw.push('\\');
                    if let Some(next) = self.advance() {
                        raw.push(next);
                    }
                }
                _ => {
                    raw.push(ch);
                    self.advance();
                }
            }
        }

        Err(ShellError::SyntaxError("Unterminated $' quote".to_string()))
    }

    fn scan_braced_param(&mut self) -> Result<WordPart> {
        // Depth- and quote-aware, so `${x:-${y}}` keeps its whole payload and `${x:-a}b}` does
        // not end early on the brace inside the quotes.
        let content = self.scan_raw_delimited('{', '}', "parameter expansion", 0)?;

        if content.starts_with('#') && content.len() > 1 {
            return Ok(WordPart::Variable {
                name: content[1..].to_string(),
                expansion_type: ParamExpansion::Length,
            });
        }

        let Some((idx, op)) = find_param_operator(&content) else {
            return Ok(WordPart::Variable {
                name: content,
                expansion_type: ParamExpansion::Normal,
            });
        };

        let name = content[..idx].to_string();
        // The operand is one word, expanded later: `${x:-$HOME}` has to yield the home directory,
        // and `${x:=$y}` has to *assign* the expanded text rather than the four characters `$y`.
        let arg = parse_word_source(&content[idx + op.len()..])?;
        let expansion_type = match op {
            ":-" => ParamExpansion::DefaultValue {
                default: arg,
                assign_if_unset: false,
                test_null: true,
            },
            ":=" => ParamExpansion::DefaultValue {
                default: arg,
                assign_if_unset: true,
                test_null: true,
            },
            ":+" => ParamExpansion::UseAlternative {
                alternative: arg,
                test_null: true,
            },
            ":?" => ParamExpansion::ErrorIfUnset {
                message: arg,
                test_null: true,
            },
            "%%" => ParamExpansion::RemoveSuffix {
                pattern: arg,
                longest: true,
            },
            "%" => ParamExpansion::RemoveSuffix {
                pattern: arg,
                longest: false,
            },
            "##" => ParamExpansion::RemovePrefix {
                pattern: arg,
                longest: true,
            },
            _ => ParamExpansion::RemovePrefix {
                pattern: arg,
                longest: false,
            },
        };

        Ok(WordPart::Variable {
            name,
            expansion_type,
        })
    }
}

/// The `${…}` operators, longest first so `%%` is never read as `%` followed by a pattern.
const PARAM_OPERATORS: &[&str] = &[":-", ":=", ":+", ":?", "%%", "##", "%", "#"];

/// Find the operator that splits a `${…}` body into name and argument.
///
/// Scanning rather than `str::find` per operator, for two reasons. Nested expansions have their
/// own operators — `${a#${b:-c}}` must split on the `#`, not on the `:-` four characters later,
/// which would cut the name in half mid-`${`. And the winner is the *leftmost* operator, not the
/// first one a fixed search order happens to hit: `${v%a:-b}` strips a suffix, it has no default.
fn find_param_operator(content: &str) -> Option<(usize, &'static str)> {
    let chars: Vec<(usize, char)> = content.char_indices().collect();
    let mut k = 0;
    let mut depth = 0usize;

    while k < chars.len() {
        let (offset, ch) = chars[k];
        match ch {
            '\\' => {
                k += 2;
                continue;
            }
            '\'' | '"' | '`' => {
                k = skip_quoted(&chars, k);
                continue;
            }
            '$' if matches!(chars.get(k + 1), Some((_, '{')) | Some((_, '('))) => {
                depth += 1;
                k += 2;
                continue;
            }
            '}' | ')' if depth > 0 => {
                depth -= 1;
            }
            // A leading `#` is the length form, `${#name}`, not a prefix strip.
            _ if depth == 0 && k > 0 => {
                if let Some(op) = PARAM_OPERATORS
                    .iter()
                    .find(|op| content[offset..].starts_with(**op))
                {
                    return Some((offset, op));
                }
            }
            _ => {}
        }
        k += 1;
    }

    None
}

/// Index just past the quoted run that starts at `k`.
fn skip_quoted(chars: &[(usize, char)], k: usize) -> usize {
    let closer = chars[k].1;
    let mut i = k + 1;
    while i < chars.len() {
        match chars[i].1 {
            // A backslash is data inside `'…'`; everywhere else it hides the next character.
            '\\' if closer != '\'' => i += 2,
            c if c == closer => return i + 1,
            _ => i += 1,
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use crate::ast::{ParamExpansion, Word, WordPart};
    use crate::lexer::{Lexer, Token};

    fn parts(src: &str) -> Vec<WordPart> {
        match Lexer::new(src).next() {
            Ok(Token::Word(w)) => w.parts,
            other => panic!("expected a word from {src:?}, got {other:?}"),
        }
    }

    fn one_part(src: &str) -> WordPart {
        let mut p = parts(src);
        assert_eq!(p.len(), 1, "expected one part from {src:?}, got {p:?}");
        p.remove(0)
    }

    // --- R2.8: `$'…'` and `$"…"` ---

    /// The finding that motivated this: `IFS=$'\n'` used to set IFS to `$`, `\`, `n`.
    #[test]
    fn ansi_c_quoting_decodes_to_one_quoted_part() {
        assert_eq!(one_part("$'\\n'"), WordPart::SingleQuoted("\n".into()));
        assert_eq!(one_part("$'a\\tb'"), WordPart::SingleQuoted("a\tb".into()));
        assert_eq!(one_part("$'\\x41'"), WordPart::SingleQuoted("A".into()));
    }

    /// Single-quoted, not literal: a decoded `*` is data and must not reach the globber.
    #[test]
    fn a_decoded_escape_is_quoted_data() {
        assert!(matches!(
            one_part("$'\\052'"),
            WordPart::SingleQuoted(ref s) if s == "*"
        ));
    }

    /// `\'` is the one escape the *scanner* has to know about: it must not end the string.
    #[test]
    fn an_escaped_quote_does_not_end_the_string() {
        assert_eq!(one_part("$'a\\'b'"), WordPart::SingleQuoted("a'b".into()));
    }

    /// bash prints `$'a\tb'` for `echo "$'a\tb'"`: inside double quotes the `$` is just a `$`.
    #[test]
    fn ansi_c_quoting_is_inert_inside_double_quotes() {
        assert_eq!(
            parts("\"$'a\\tb'\""),
            vec![WordPart::DoubleQuoted(vec![
                WordPart::Literal("$".into()),
                WordPart::Literal("'a\\tb'".into()),
            ])]
        );
    }

    /// `$"…"` is a translatable string; with no catalogue it is an ordinary double-quoted run.
    #[test]
    fn dollar_double_quote_is_a_plain_double_quoted_run() {
        assert_eq!(
            one_part("$\"a $b\""),
            WordPart::DoubleQuoted(vec![
                WordPart::Literal("a ".into()),
                WordPart::Variable {
                    name: "b".into(),
                    expansion_type: ParamExpansion::Normal,
                },
            ])
        );
    }

    // --- R2.6: brace depth and quote state ---

    /// `${x:-${y}}` used to stop at the first `}`, leaving `}` behind as literal text.
    #[test]
    fn a_braced_payload_may_contain_a_braced_expansion() {
        assert_eq!(
            one_part("${x:-${y}}"),
            WordPart::Variable {
                name: "x".into(),
                expansion_type: ParamExpansion::DefaultValue {
                    default: Word {
                        parts: vec![WordPart::Variable {
                            name: "y".into(),
                            expansion_type: ParamExpansion::Normal,
                        }]
                    },
                    assign_if_unset: false,
                    test_null: true,
                },
            }
        );
    }

    /// The operator search has to skip nested expansions: the `:-` here belongs to the inner
    /// `${b:-c}`, so splitting on it would cut the name off mid-`${`.
    #[test]
    fn a_nested_operator_does_not_win() {
        let WordPart::Variable {
            name,
            expansion_type,
        } = one_part("${a#${b:-c}}")
        else {
            panic!("expected a variable");
        };
        assert_eq!(name, "a");
        assert!(matches!(
            expansion_type,
            ParamExpansion::RemovePrefix { longest: false, .. }
        ));
    }

    /// `%%` beats `%`, and the leftmost operator beats a later one of a different kind.
    #[test]
    fn the_leftmost_longest_operator_wins() {
        assert!(matches!(
            one_part("${v%%.*}"),
            WordPart::Variable {
                expansion_type: ParamExpansion::RemoveSuffix { longest: true, .. },
                ..
            }
        ));
        assert!(matches!(
            one_part("${v%a:-b}"),
            WordPart::Variable {
                expansion_type: ParamExpansion::RemoveSuffix { longest: false, .. },
                ..
            }
        ));
    }

    /// A `}` inside quotes, or inside a nested substitution, is data.
    #[test]
    fn a_quoted_brace_does_not_close_the_expansion() {
        assert_eq!(
            one_part("${x:-'a}b'}"),
            WordPart::Variable {
                name: "x".into(),
                expansion_type: ParamExpansion::DefaultValue {
                    default: Word {
                        parts: vec![WordPart::SingleQuoted("a}b".into())]
                    },
                    assign_if_unset: false,
                    test_null: true,
                },
            }
        );
        assert!(matches!(
            one_part("${x:-$(echo })}"),
            WordPart::Variable {
                expansion_type: ParamExpansion::DefaultValue { .. },
                ..
            }
        ));
    }

    /// The payload is a word, not text: `${x:-$HOME}` has to expand later, not print `$HOME`.
    #[test]
    fn a_payload_is_parsed_as_a_word() {
        let WordPart::Variable {
            expansion_type: ParamExpansion::DefaultValue { default, .. },
            ..
        } = one_part("${x:-$(pwd)/sub}")
        else {
            panic!("expected a default-value expansion");
        };
        assert_eq!(
            default.parts,
            vec![
                WordPart::CommandSubstitution("pwd".into()),
                WordPart::Literal("/sub".into()),
            ]
        );
    }
}
