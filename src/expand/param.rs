//! Parameter expansion: `$x`, `${x}`, and every `${x<op>word}` operator.
//!
//! Two rules shape this module.
//!
//! The first is that an *operand is a word, not a string*. `${x:-$HOME}` has to expand its
//! default and `${x:=$y}` has to persist the expanded text, so every payload arrives as a
//! [`Word`] and is expanded here, with quote removal, at the moment the operator needs it.
//!
//! The second is that an unrecognised `${...}` body is an **error**. The lexer hands anything it
//! cannot parse through as a parameter *name*; looking that up would answer `""` for every form
//! the shell does not implement, and a wrong-but-quiet empty string is precisely how nine
//! separate unimplemented operators went unnoticed. Rejecting a body that is not a name turns
//! that fallback into a diagnostic instead.

mod pattern;

use crate::ast::{ParamExpansion, ReplaceScope, Word};
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::expand::arithmetic::eval_arithmetic;
use crate::expand::word::{Field, Origin, Run, expand_word_to_string};

/// Expand one parameter reference into the fields it contributes.
///
/// `in_quotes` is the enclosing double-quote context: it decides both whether the result is
/// eligible for field splitting and, for `$@`, whether the positionals stay apart.
pub fn expand_param(
    env: &mut Environment,
    name: &str,
    expansion_type: &ParamExpansion,
    in_quotes: bool,
) -> Result<Vec<Field>> {
    let origin = if in_quotes {
        Origin::Quoted
    } else {
        Origin::Expanded
    };

    // `$@` and `$*` are the only parameters that can be more than one field, and only `$@` keeps
    // them apart. Every other form has a single string as its value, so it takes the path below.
    if matches!(expansion_type, ParamExpansion::Normal) && matches!(name, "@" | "*") {
        return Ok(positional_fields(env, name, origin));
    }

    let text = expand_to_string(env, name, expansion_type)?;
    Ok(vec![vec![Run::new(text, origin)]])
}

/// The single string a `${...}` reference stands for.
fn expand_to_string(
    env: &mut Environment,
    name: &str,
    expansion_type: &ParamExpansion,
) -> Result<String> {
    if !is_param_name(name) {
        return Err(ShellError::ExpansionError(format!(
            "${{{name}}}: bad substitution"
        )));
    }

    let val = env.get_param(name);
    let text = match expansion_type {
        ParamExpansion::Normal => val.unwrap_or_default(),

        // `${#@}` and `${#*}` count the positionals rather than measuring the string they would
        // have joined into. `${#}` never reaches here: it is `$#`, a plain reference.
        ParamExpansion::Length => match name {
            "@" | "*" => env.get_positional().len().to_string(),
            // Characters, not bytes — `${#v}` on a UTF-8 value must not report its encoding.
            _ => val.map_or_else(|| "0".to_string(), |v| v.chars().count().to_string()),
        },

        ParamExpansion::DefaultValue {
            default,
            assign_if_unset,
            test_null,
        } => {
            if is_present(&val, *test_null) {
                val.unwrap_or_default()
            } else {
                let text = expand_word_to_string(env, default)?;
                // Assigning the *expanded* text is the whole point: `${x:=$y}` used to persist
                // the two characters `$y` into `x`.
                if *assign_if_unset {
                    env.set_var(name, &text, false);
                }
                text
            }
        }

        ParamExpansion::UseAlternative {
            alternative,
            test_null,
        } => {
            if is_present(&val, *test_null) {
                expand_word_to_string(env, alternative)?
            } else {
                String::new()
            }
        }

        ParamExpansion::ErrorIfUnset { message, test_null } => {
            if is_present(&val, *test_null) {
                val.unwrap_or_default()
            } else {
                let msg = expand_word_to_string(env, message)?;
                let msg = if msg.is_empty() {
                    "parameter null or not set".to_string()
                } else {
                    msg
                };
                return Err(ShellError::ExpansionError(format!("{name}: {msg}")));
            }
        }

        ParamExpansion::RemovePrefix {
            pattern: word,
            longest,
        } => {
            let pat = expand_word_to_string(env, word)?;
            let value = val.unwrap_or_default();
            pattern::remove_prefix(&value, &pat, *longest)
        }

        ParamExpansion::RemoveSuffix {
            pattern: word,
            longest,
        } => {
            let pat = expand_word_to_string(env, word)?;
            let value = val.unwrap_or_default();
            pattern::remove_suffix(&value, &pat, *longest)
        }

        ParamExpansion::Substring { offset, length } => {
            let start = eval_operand(env, offset)?;
            let count = match length {
                Some(word) => Some(eval_operand(env, word)?),
                None => None,
            };
            let value = val.unwrap_or_default();
            pattern::substring(&value, start, count)
                .map_err(|n| ShellError::ExpansionError(format!("{n}: substring expression < 0")))?
        }

        ParamExpansion::Replace {
            pattern: pat_word,
            replacement,
            scope,
        } => {
            let pat = expand_word_to_string(env, pat_word)?;
            let rep = expand_word_to_string(env, replacement)?;
            let value = val.unwrap_or_default();
            match scope {
                ReplaceScope::First => pattern::replace(&value, &pat, &rep, false),
                ReplaceScope::All => pattern::replace(&value, &pat, &rep, true),
                ReplaceScope::Prefix => pattern::replace_prefix(&value, &pat, &rep),
                ReplaceScope::Suffix => pattern::replace_suffix(&value, &pat, &rep),
            }
        }

        ParamExpansion::CaseConvert {
            pattern: selector,
            upper,
            all,
        } => {
            let selector = match selector {
                Some(word) => Some(expand_word_to_string(env, word)?),
                None => None,
            };
            let value = val.unwrap_or_default();
            pattern::convert_case(&value, selector.as_deref(), *upper, *all)
        }

        // `${!name}` reads `name`'s value and expands *that* parameter. Only the second lookup
        // may come up empty: bash makes a `name` that does not *hold a name* a fatal expansion
        // error, and it names a different culprit depending on which step failed.
        ParamExpansion::Indirect => match val {
            None => {
                return Err(ShellError::ExpansionError(format!(
                    "{name}: invalid indirect expansion"
                )));
            }
            // An empty value lands here too, which is why the message quotes nothing.
            Some(target) if !is_param_name(&target) => {
                return Err(ShellError::ExpansionError(format!(
                    "{target}: invalid variable name"
                )));
            }
            Some(target) => env.get_param(&target).unwrap_or_default(),
        },
    };

    Ok(text)
}

/// Whether the parameter counts as "set" for the `${x-d}` family.
///
/// The `:` forms treat a set-but-empty parameter as absent, the colon-less ones test only for
/// unset — which is the entire difference between `x=; ${x:-d}` (`d`) and `x=; ${x-d}` (empty).
fn is_present(val: &Option<String>, test_null: bool) -> bool {
    match val {
        Some(v) => !(test_null && v.is_empty()),
        None => false,
    }
}

/// A substring operand is an arithmetic expression, so `${v:i+1:n-1}` works.
///
/// An absent operand is 0 rather than an error: `${v: }` is degenerate but not fatal, and the
/// arithmetic evaluator rejects an empty expression.
fn eval_operand(env: &mut Environment, word: &Word) -> Result<i64> {
    let text = expand_word_to_string(env, word)?;
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    eval_arithmetic(env, text)
}

/// Can `name` be looked up as a parameter at all?
///
/// This is the guard that turns an unimplemented `${...}` form into a diagnostic. The lexer's
/// fallback for a body it cannot parse is to treat the whole body as a name, and every body that
/// contains an operator fails here — `v:2:3`, `v/x/y`, `v^^` are not names.
fn is_param_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    // The special parameters, plus `$-`, are each a single character and none is a name.
    if name.chars().count() == 1 && matches!(first, '?' | '$' | '!' | '#' | '*' | '@' | '-') {
        return true;
    }
    // `${12}` is the twelfth positional: all digits, or a name.
    if first.is_ascii_digit() {
        return name.chars().all(|c| c.is_ascii_digit());
    }
    (first.is_alphabetic() || first == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// `$@` and `$*` as fields.
///
/// `"$@"` is one field per positional — the difference between forwarding `"a b" c` as two
/// arguments and as three — and *no* field when there are none, so `cmd "$@"` with nothing set
/// runs `cmd` rather than `cmd ""`. `$*` is always exactly one field, joined by the first
/// character of IFS.
fn positional_fields(env: &Environment, name: &str, origin: Origin) -> Vec<Field> {
    let params = env.get_positional();
    if name == "@" {
        return params
            .iter()
            .map(|p| vec![Run::new(p.clone(), origin)])
            .collect();
    }
    vec![vec![Run::new(params.join(&env.ifs_separator()), origin)]]
}

#[cfg(test)]
mod tests {
    use super::{expand_to_string, is_param_name, positional_fields};
    use crate::ast::{ParamExpansion, ReplaceScope, Word, WordPart};
    use crate::env::Environment;
    use crate::expand::word::{Origin, field_text};

    fn texts(env: &Environment, name: &str, origin: Origin) -> Vec<String> {
        positional_fields(env, name, origin)
            .iter()
            .map(|f| field_text(f))
            .collect()
    }

    /// Expand `${name<op>}` against an environment holding `vars`.
    fn expand(vars: &[(&str, &str)], name: &str, op: ParamExpansion) -> Result<String, String> {
        let mut env = Environment::new();
        for (k, v) in vars {
            env.set_var(k, v, false);
        }
        expand_to_string(&mut env, name, &op).map_err(|e| e.to_string())
    }

    fn lit(s: &str) -> Word {
        Word::from_literal(s)
    }

    /// A word that expands `$other` — the payload shape R2.6 exists for.
    fn var_word(other: &str) -> Word {
        Word {
            parts: vec![WordPart::Variable {
                name: other.to_string(),
                expansion_type: ParamExpansion::Normal,
            }],
        }
    }

    #[test]
    fn at_is_one_field_per_positional() {
        let mut env = Environment::new();
        env.set_positional(vec!["a b".into(), "c".into()]);
        assert_eq!(texts(&env, "@", Origin::Quoted), vec!["a b", "c"]);
    }

    #[test]
    fn at_with_no_positionals_is_no_field() {
        let mut env = Environment::new();
        env.set_positional(Vec::new());
        assert!(positional_fields(&env, "@", Origin::Quoted).is_empty());
    }

    #[test]
    fn star_joins_with_the_first_ifs_character() {
        let mut env = Environment::new();
        env.set_positional(vec!["a".into(), "b".into()]);
        env.set_var("IFS", "-+", false);
        assert_eq!(texts(&env, "*", Origin::Quoted), vec!["a-b"]);
    }

    /// An unset IFS means the default, whose first character is a space.
    #[test]
    fn star_defaults_to_a_space() {
        let mut env = Environment::new();
        env.set_positional(vec!["a".into(), "b".into()]);
        env.unset_var("IFS");
        assert_eq!(texts(&env, "*", Origin::Quoted), vec!["a b"]);
    }

    /// With IFS empty there is no separator at all, so `$*` concatenates.
    #[test]
    fn star_with_empty_ifs_concatenates() {
        let mut env = Environment::new();
        env.set_positional(vec!["a".into(), "b".into()]);
        env.set_var("IFS", "", false);
        assert_eq!(texts(&env, "*", Origin::Quoted), vec!["ab"]);
    }

    /// The regression this module's second rule exists for: an unimplemented form used to be
    /// looked up as a variable named after the whole brace body, and answer `""`.
    #[test]
    fn an_unparsed_brace_body_is_an_error_not_an_empty_string() {
        for body in ["v:2:3", "v/x/y", "v^^", "v@Q", "!v", "a b", ""] {
            let got = expand(&[], body, ParamExpansion::Normal);
            assert!(
                got.is_err(),
                "${{{body}}} expanded to {got:?}, not an error"
            );
        }
    }

    #[test]
    fn a_real_name_is_still_a_name() {
        assert!(is_param_name("HOME"));
        assert!(is_param_name("_x1"));
        assert!(is_param_name("12"));
        assert!(is_param_name("@"));
        assert!(is_param_name("#"));
        assert!(!is_param_name(""));
        assert!(!is_param_name("1a"));
        assert!(!is_param_name("v-d"));
    }

    #[test]
    fn length_counts_characters_and_positionals() {
        let len = |value: &str| expand(&[("s", value)], "s", ParamExpansion::Length);
        assert_eq!(len("hello"), Ok("5".into()));
        // Characters, not bytes.
        assert_eq!(len("héllo"), Ok("5".into()));
        assert_eq!(expand(&[], "nope", ParamExpansion::Length), Ok("0".into()));

        let mut env = Environment::new();
        env.set_positional(vec!["a".into(), "bb".into(), "ccc".into()]);
        // `${#@}` is the count, not the width of the joined string.
        let count = expand_to_string(&mut env, "@", &ParamExpansion::Length).unwrap();
        assert_eq!(count, "3");
        // And `${#}` is `$#`, which is the same number.
        let count = expand_to_string(&mut env, "#", &ParamExpansion::Normal).unwrap();
        assert_eq!(count, "3");
    }

    /// The colon-less forms test only for unset; the `:` forms also reject a null value.
    #[test]
    fn the_colon_decides_whether_null_counts_as_set() {
        // `x` is set but null throughout: `${x:-d}` substitutes, `${x-d}` does not.
        for (test_null, expected) in [(true, "d"), (false, "")] {
            let op = ParamExpansion::DefaultValue {
                default: lit("d"),
                assign_if_unset: false,
                test_null,
            };
            assert_eq!(expand(&[("x", "")], "x", op), Ok(expected.to_string()));
        }
        for test_null in [true, false] {
            let op = ParamExpansion::UseAlternative {
                alternative: lit("s"),
                test_null,
            };
            let expected = if test_null { "" } else { "s" };
            assert_eq!(expand(&[("x", "")], "x", op), Ok(expected.to_string()));
        }
    }

    /// R2.6: the payload is a word, so it expands.
    #[test]
    fn a_default_payload_is_expanded() {
        let op = ParamExpansion::DefaultValue {
            default: var_word("d"),
            assign_if_unset: false,
            test_null: true,
        };
        assert_eq!(expand(&[("d", "/tmp/x")], "v", op), Ok("/tmp/x".into()));
    }

    /// R2.6: and the assigning form persists the *expanded* text, not the source.
    #[test]
    fn the_assigning_form_stores_the_expanded_text() {
        let mut env = Environment::new();
        env.set_var("y", "value", false);
        let op = ParamExpansion::DefaultValue {
            default: var_word("y"),
            assign_if_unset: true,
            test_null: true,
        };
        assert_eq!(expand_to_string(&mut env, "x", &op).unwrap(), "value");
        assert_eq!(env.get_param("x"), Some("value".to_string()));
    }

    #[test]
    fn error_if_unset_reports_the_expanded_message() {
        let op = ParamExpansion::ErrorIfUnset {
            message: var_word("m"),
            test_null: true,
        };
        let err = expand(&[("m", "is unset")], "v", op).unwrap_err();
        assert!(err.contains("is unset"), "{err}");
    }

    #[test]
    fn prefix_and_suffix_operators_use_patterns() {
        let p = "/usr/local/lib/libfoo.so";
        let strip = |pattern: &str, longest, prefix| {
            let op = if prefix {
                ParamExpansion::RemovePrefix {
                    pattern: lit(pattern),
                    longest,
                }
            } else {
                ParamExpansion::RemoveSuffix {
                    pattern: lit(pattern),
                    longest,
                }
            };
            expand(&[("p", p)], "p", op).unwrap()
        };
        assert_eq!(strip("*/", true, true), "libfoo.so");
        assert_eq!(strip("/*", false, false), "/usr/local/lib");
        assert_eq!(strip(".*", false, false), "/usr/local/lib/libfoo");
    }

    /// The pattern is a word too, so `${p##$sep}` works.
    #[test]
    fn a_strip_pattern_is_expanded() {
        let op = ParamExpansion::RemovePrefix {
            pattern: var_word("sep"),
            longest: true,
        };
        let vars = [("p", "a/b/c"), ("sep", "*/")];
        assert_eq!(expand(&vars, "p", op), Ok("c".into()));
    }

    #[test]
    fn substring_reads_arithmetic_operands() {
        let op = ParamExpansion::Substring {
            offset: lit("1+1"),
            length: Some(lit("3")),
        };
        assert_eq!(expand(&[("v", "abcdefgh")], "v", op), Ok("cde".into()));

        let op = ParamExpansion::Substring {
            offset: lit(" -3"),
            length: None,
        };
        assert_eq!(expand(&[("v", "abcdefgh")], "v", op), Ok("fgh".into()));

        // bash makes a window that ends before it starts a fatal error, not an empty string.
        let op = ParamExpansion::Substring {
            offset: lit("2"),
            length: Some(lit("-9")),
        };
        assert!(expand(&[("v", "abcdefgh")], "v", op).is_err());
    }

    #[test]
    fn replacement_covers_every_scope() {
        let scopes = [
            (ReplaceScope::First, "-", "+", "a+b-c"),
            (ReplaceScope::All, "-", "+", "a+b+c"),
            (ReplaceScope::Prefix, "a", "A", "A-b-c"),
            (ReplaceScope::Suffix, "c", "C", "a-b-C"),
        ];
        for (scope, pat, rep, expected) in scopes {
            let op = ParamExpansion::Replace {
                pattern: lit(pat),
                replacement: lit(rep),
                scope,
            };
            assert_eq!(expand(&[("v", "a-b-c")], "v", op), Ok(expected.to_string()));
        }
    }

    #[test]
    fn case_conversion_covers_both_directions() {
        let cases = [
            (true, true, "hello", "HELLO"),
            (true, false, "hello", "Hello"),
            (false, true, "WORLD", "world"),
            (false, false, "WORLD", "wORLD"),
        ];
        for (upper, all, value, expected) in cases {
            let op = ParamExpansion::CaseConvert {
                pattern: None,
                upper,
                all,
            };
            assert_eq!(expand(&[("v", value)], "v", op), Ok(expected.to_string()));
        }
    }

    /// `rush_ref`, not `name`: [`Environment::new`] inherits the real environment, and a plain
    /// word like `name` is genuinely exported by some development shells — which would make the
    /// unset case below pass or fail depending on who ran the tests.
    #[test]
    fn indirection_follows_the_named_parameter() {
        let vars = [("rush_target", "payload"), ("rush_ref", "rush_target")];
        let got = expand(&vars, "rush_ref", ParamExpansion::Indirect);
        assert_eq!(got, Ok("payload".into()));
        // Only the *inner* parameter may be unset; that is an empty string, not an error.
        let vars = [("rush_ref", "rush_nosuchvar")];
        let got = expand(&vars, "rush_ref", ParamExpansion::Indirect);
        assert_eq!(got, Ok(String::new()));
        // bash aborts the expansion when the referring parameter is unset, or holds anything
        // that is not a name — including the empty string.
        for vars in [vec![], vec![("rush_ref", "")], vec![("rush_ref", "a b")]] {
            let got = expand(&vars, "rush_ref", ParamExpansion::Indirect);
            assert!(got.is_err(), "{vars:?} expanded to {got:?}");
        }
    }
}
