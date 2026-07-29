//! Parsing the body of a `${…}` parameter expansion.
//!
//! The lexer hands this module the raw text between `${` and its matching `}` and gets back the
//! [`WordPart::Variable`] it stands for. Splitting the body is the whole job, and it is harder
//! than it looks for three reasons.
//!
//! *The operator is the leftmost one, not the first one a search order happens to find.*
//! `${v%a:-b}` strips a suffix; it has no default. And the search has to skip nested expansions,
//! quotes and backslashes, so `${a#${b:-c}}` splits on the `#` rather than cutting the name in
//! half mid-`${`.
//!
//! *A colon is two different operators.* `${v:-d}` is a default and `${v:2:3}` is a substring, so
//! the `:-` family has to be tried before a bare `:`. That is also why bash needs `${v: -1}` or
//! `${v:(-1)}` for a negative offset — and why this module needs nothing extra to support them.
//!
//! *An unrecognised body must stay an error.* Anything this module cannot split is passed through
//! as a parameter *name*, and `crate::expand::param` rejects a name that is not one. The silent
//! empty string that fallback used to produce is what hid nine unimplemented operators.

use super::quoting::parse_word_source;
use crate::ast::{ParamExpansion, ReplaceScope, WordPart};
use crate::error::Result;
use scan::{Nesting, find_param_operator, split_top_level};

mod scan;

/// Turn the raw body of a `${…}` into the word part it denotes.
pub(super) fn parse_braced_body(content: &str) -> Result<WordPart> {
    if let Some(part) = prefix_form(content) {
        return Ok(part);
    }

    let Some((idx, op)) = find_param_operator(content) else {
        return Ok(WordPart::Variable {
            name: content.to_string(),
            expansion_type: ParamExpansion::Normal,
        });
    };

    Ok(WordPart::Variable {
        name: content[..idx].to_string(),
        expansion_type: operator_expansion(op, &content[idx + op.len()..])?,
    })
}

/// The two operators that sit *before* the name: `${#name}` and `${!name}`.
///
/// A lone `#` or `!` is not one of them. `${#}` is `$#` and `${!}` is `$!`, plain references to a
/// special parameter, so the marker only counts when a name follows it.
///
/// `${!prefix*}` — bash's name-listing form — reaches here as the name `prefix*`, which is not a
/// parameter name and so becomes a `bad substitution` error rather than a wrong answer.
fn prefix_form(content: &str) -> Option<WordPart> {
    let mut chars = content.chars();
    let expansion_type = match chars.next()? {
        '#' => ParamExpansion::Length,
        '!' => ParamExpansion::Indirect,
        _ => return None,
    };
    let name = chars.as_str();
    if name.is_empty() {
        return None;
    }
    Some(WordPart::Variable {
        name: name.to_string(),
        expansion_type,
    })
}

/// Build the expansion for `op` from the text that follows it.
///
/// Every operand stays a [`crate::ast::Word`]: `${x:-$HOME}` has to expand its default, and
/// `${v/$sep/-}` has to expand its pattern, both at use time rather than here.
fn operator_expansion(op: &str, rest: &str) -> Result<ParamExpansion> {
    Ok(match op {
        // A substring operand is arithmetic, so parentheses nest and `${v:(-1)}` splits correctly.
        ":" => {
            let (offset, length) = split_top_level(rest, ':', Nesting::Arithmetic);
            ParamExpansion::Substring {
                offset: parse_word_source(offset)?,
                length: length.map(parse_word_source).transpose()?,
            }
        }

        // `${v/pat}` with no second `/` deletes the match, so an absent replacement is empty
        // rather than an error. Parentheses are ordinary pattern data here, unlike above.
        "/" | "//" | "/#" | "/%" => {
            let (pattern, replacement) = split_top_level(rest, '/', Nesting::Expansion);
            ParamExpansion::Replace {
                pattern: parse_word_source(pattern)?,
                replacement: parse_word_source(replacement.unwrap_or(""))?,
                scope: match op {
                    "//" => ReplaceScope::All,
                    "/#" => ReplaceScope::Prefix,
                    "/%" => ReplaceScope::Suffix,
                    _ => ReplaceScope::First,
                },
            }
        }

        // The optional operand selects which characters are eligible; absent means all of them.
        "^" | "^^" | "," | ",," => ParamExpansion::CaseConvert {
            pattern: if rest.is_empty() {
                None
            } else {
                Some(parse_word_source(rest)?)
            },
            upper: op.starts_with('^'),
            all: op.chars().count() == 2,
        },

        _ => {
            let arg = parse_word_source(rest)?;
            // A leading `:` is not a different operator, only a different notion of "absent":
            // the `:` forms treat a set-but-empty parameter as unset, the colon-less ones do not.
            let test_null = op.starts_with(':');
            match op.strip_prefix(':').unwrap_or(op) {
                "-" => ParamExpansion::DefaultValue {
                    default: arg,
                    assign_if_unset: false,
                    test_null,
                },
                "=" => ParamExpansion::DefaultValue {
                    default: arg,
                    assign_if_unset: true,
                    test_null,
                },
                "+" => ParamExpansion::UseAlternative {
                    alternative: arg,
                    test_null,
                },
                "?" => ParamExpansion::ErrorIfUnset {
                    message: arg,
                    test_null,
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
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::ast::{ParamExpansion, ReplaceScope, Word, WordPart};
    use crate::lexer::{Lexer, Token};

    fn one_part(src: &str) -> WordPart {
        let mut parts = match Lexer::new(src).next() {
            Ok(Token::Word(w)) => w.parts,
            other => panic!("expected a word from {src:?}, got {other:?}"),
        };
        assert_eq!(parts.len(), 1, "expected one part from {src:?}");
        parts.remove(0)
    }

    /// The `expansion_type` of a `${…}`, with the name checked along the way.
    fn op_of(src: &str, expect_name: &str) -> ParamExpansion {
        let WordPart::Variable {
            name,
            expansion_type,
        } = one_part(src)
        else {
            panic!("expected a variable from {src:?}");
        };
        assert_eq!(name, expect_name, "name from {src:?}");
        expansion_type
    }

    fn lit(s: &str) -> Word {
        Word::from_literal(s)
    }

    // --- R2.6: brace depth, quote state, and the leftmost operator ---

    /// `${x:-${y}}` used to stop at the first `}`, leaving `}` behind as literal text.
    #[test]
    fn a_braced_payload_may_contain_a_braced_expansion() {
        assert_eq!(
            op_of("${x:-${y}}", "x"),
            ParamExpansion::DefaultValue {
                default: Word {
                    parts: vec![WordPart::Variable {
                        name: "y".into(),
                        expansion_type: ParamExpansion::Normal,
                    }]
                },
                assign_if_unset: false,
                test_null: true,
            }
        );
    }

    /// The operator search has to skip nested expansions: the `:-` here belongs to the inner
    /// `${b:-c}`, so splitting on it would cut the name off mid-`${`.
    #[test]
    fn a_nested_operator_does_not_win() {
        assert!(matches!(
            op_of("${a#${b:-c}}", "a"),
            ParamExpansion::RemovePrefix { longest: false, .. }
        ));
    }

    /// `%%` beats `%`, and the leftmost operator beats a later one of a different kind.
    #[test]
    fn the_leftmost_longest_operator_wins() {
        assert!(matches!(
            op_of("${v%%.*}", "v"),
            ParamExpansion::RemoveSuffix { longest: true, .. }
        ));
        assert!(matches!(
            op_of("${v%a:-b}", "v"),
            ParamExpansion::RemoveSuffix { longest: false, .. }
        ));
    }

    /// A `}` inside quotes, or inside a nested substitution, is data.
    #[test]
    fn a_quoted_brace_does_not_close_the_expansion() {
        assert_eq!(
            op_of("${x:-'a}b'}", "x"),
            ParamExpansion::DefaultValue {
                default: Word {
                    parts: vec![WordPart::SingleQuoted("a}b".into())]
                },
                assign_if_unset: false,
                test_null: true,
            }
        );
        assert!(matches!(
            op_of("${x:-$(echo })}", "x"),
            ParamExpansion::DefaultValue { .. }
        ));
    }

    /// The payload is a word, not text: `${x:-$HOME}` has to expand later, not print `$HOME`.
    #[test]
    fn a_payload_is_parsed_as_a_word() {
        let ParamExpansion::DefaultValue { default, .. } = op_of("${x:-$(pwd)/sub}", "x") else {
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

    // --- R2.4: the forms the lexer used to drop ---

    /// The prefix operators, and the lone `#`/`!` that are special parameters instead.
    #[test]
    fn the_prefix_operators_need_a_name_after_them() {
        assert_eq!(op_of("${#v}", "v"), ParamExpansion::Length);
        assert_eq!(op_of("${!v}", "v"), ParamExpansion::Indirect);
        // `${#}` is `$#` and `${!}` is `$!` — a marker with nothing after it is the parameter.
        assert_eq!(op_of("${#}", "#"), ParamExpansion::Normal);
        assert_eq!(op_of("${!}", "!"), ParamExpansion::Normal);
        assert_eq!(op_of("${#@}", "@"), ParamExpansion::Length);
    }

    /// `${!prefix*}` is bash's name-listing form, which rush does not implement. It must reach
    /// the expander as a non-name so it errors, not as something that quietly expands.
    #[test]
    fn the_name_listing_form_is_left_as_a_bad_name() {
        assert_eq!(op_of("${!pre*}", "pre*"), ParamExpansion::Indirect);
    }

    #[test]
    fn the_colon_less_forms_only_test_for_unset() {
        assert_eq!(
            op_of("${v-d}", "v"),
            ParamExpansion::DefaultValue {
                default: lit("d"),
                assign_if_unset: false,
                test_null: false,
            }
        );
        assert_eq!(
            op_of("${v=d}", "v"),
            ParamExpansion::DefaultValue {
                default: lit("d"),
                assign_if_unset: true,
                test_null: false,
            }
        );
        assert_eq!(
            op_of("${v+s}", "v"),
            ParamExpansion::UseAlternative {
                alternative: lit("s"),
                test_null: false,
            }
        );
        assert_eq!(
            op_of("${v?e}", "v"),
            ParamExpansion::ErrorIfUnset {
                message: lit("e"),
                test_null: false,
            }
        );
    }

    /// The `:` family still wins over the bare `:`, which is what makes `${v:-1}` a default and
    /// not an offset of -1.
    #[test]
    fn the_colon_family_beats_the_bare_colon() {
        assert!(matches!(
            op_of("${v:-1}", "v"),
            ParamExpansion::DefaultValue { .. }
        ));
        assert!(matches!(
            op_of("${v:=1}", "v"),
            ParamExpansion::DefaultValue {
                assign_if_unset: true,
                ..
            }
        ));
        assert!(matches!(
            op_of("${v:+1}", "v"),
            ParamExpansion::UseAlternative { .. }
        ));
        assert!(matches!(
            op_of("${v:?1}", "v"),
            ParamExpansion::ErrorIfUnset { .. }
        ));
    }

    #[test]
    fn substring_splits_offset_from_length() {
        assert_eq!(
            op_of("${v:2:3}", "v"),
            ParamExpansion::Substring {
                offset: lit("2"),
                length: Some(lit("3")),
            }
        );
        assert_eq!(
            op_of("${v:2}", "v"),
            ParamExpansion::Substring {
                offset: lit("2"),
                length: None,
            }
        );
    }

    /// A negative offset needs a space or parens, exactly as in bash, because `:-` is a default.
    #[test]
    fn a_negative_offset_needs_a_space_or_parens() {
        assert_eq!(
            op_of("${v: -3}", "v"),
            ParamExpansion::Substring {
                offset: lit(" -3"),
                length: None,
            }
        );
        assert_eq!(
            op_of("${v:(-3):2}", "v"),
            ParamExpansion::Substring {
                offset: lit("(-3)"),
                length: Some(lit("2")),
            }
        );
    }

    /// A `:` inside a nested expansion belongs to it, not to the substring split.
    #[test]
    fn substring_operands_may_nest() {
        assert_eq!(
            op_of("${v:${a:-1}:$((b+1))}", "v"),
            ParamExpansion::Substring {
                offset: Word {
                    parts: vec![WordPart::Variable {
                        name: "a".into(),
                        expansion_type: ParamExpansion::DefaultValue {
                            default: lit("1"),
                            assign_if_unset: false,
                            test_null: true,
                        },
                    }]
                },
                length: Some(Word {
                    parts: vec![WordPart::Arithmetic("b+1".into())]
                }),
            }
        );
    }

    #[test]
    fn replacement_covers_every_scope() {
        let scopes = [
            ("${v/a/b}", ReplaceScope::First),
            ("${v//a/b}", ReplaceScope::All),
            ("${v/#a/b}", ReplaceScope::Prefix),
            ("${v/%a/b}", ReplaceScope::Suffix),
        ];
        for (src, want) in scopes {
            assert_eq!(
                op_of(src, "v"),
                ParamExpansion::Replace {
                    pattern: lit("a"),
                    replacement: lit("b"),
                    scope: want,
                }
            );
        }
    }

    /// Only the *first* separator splits: a later `/` is part of the replacement text.
    #[test]
    fn a_slash_in_the_replacement_is_data() {
        assert_eq!(
            op_of("${v/x//y}", "v"),
            ParamExpansion::Replace {
                pattern: lit("x"),
                replacement: lit("/y"),
                scope: ReplaceScope::First,
            }
        );
    }

    /// With no second `/` the match is deleted, so the replacement is empty rather than absent.
    #[test]
    fn a_missing_replacement_is_empty() {
        assert_eq!(
            op_of("${v/x}", "v"),
            ParamExpansion::Replace {
                pattern: lit("x"),
                replacement: Word::default(),
                scope: ReplaceScope::First,
            }
        );
    }

    /// The pattern is a word, so `${v/$sep/-}` searches for the *value* of `sep`, and a `/`
    /// inside a nested expansion does not split the operands.
    #[test]
    fn a_replacement_pattern_is_a_word() {
        assert_eq!(
            op_of("${v/${a:-/}/-}", "v"),
            ParamExpansion::Replace {
                pattern: Word {
                    parts: vec![WordPart::Variable {
                        name: "a".into(),
                        expansion_type: ParamExpansion::DefaultValue {
                            default: lit("/"),
                            assign_if_unset: false,
                            test_null: true,
                        },
                    }]
                },
                replacement: lit("-"),
                scope: ReplaceScope::First,
            }
        );
    }

    #[test]
    fn case_conversion_reads_both_directions_and_its_selector() {
        let cases = [
            ("${v^}", true, false),
            ("${v^^}", true, true),
            ("${v,}", false, false),
            ("${v,,}", false, true),
        ];
        for (src, upper, all) in cases {
            assert_eq!(
                op_of(src, "v"),
                ParamExpansion::CaseConvert {
                    pattern: None,
                    upper,
                    all,
                }
            );
        }
        assert_eq!(
            op_of("${v^^[aeiou]}", "v"),
            ParamExpansion::CaseConvert {
                pattern: Some(lit("[aeiou]")),
                upper: true,
                all: true,
            }
        );
    }

    /// The property the whole module protects: a body it cannot split stays a *name*, so the
    /// expander rejects it. `${v@Q}` is a real bash operator rush does not implement.
    #[test]
    fn an_unknown_operator_is_left_as_a_bad_name() {
        assert_eq!(op_of("${v@Q}", "v@Q"), ParamExpansion::Normal);
        assert_eq!(op_of("${a b}", "a b"), ParamExpansion::Normal);
    }
}
