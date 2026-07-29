//! What a `${…}` operator *means*.
//!
//! The counterpart to `scan`, which only finds where an operator is: given the operator text and
//! the argument after it, this builds the [`ParamExpansion`] the expander will act on. Kept apart
//! so the positional hazards (nesting, quoting, the leftmost-longest rule) stay in one file and
//! the operator table stays in another.

use super::scan::{Nesting, split_top_level};
use crate::ast::{ParamExpansion, ReplaceScope};
use crate::error::Result;
use crate::lexer::quoting::parse_word_source;

/// Build the expansion for `op` from the text that follows it.
///
/// Every operand stays a [`crate::ast::Word`]: `${x:-$HOME}` has to expand its default, and
/// `${v/$sep/-}` has to expand its pattern, both at use time rather than here.
pub(super) fn operator_expansion(op: &str, rest: &str) -> Result<ParamExpansion> {
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
