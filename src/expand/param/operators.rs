//! Applying a `${…}` operator to the value it was given.
//!
//! Split from the module that *finds* the value so that a scalar and an array element share one
//! implementation of every operator: `${v:-d}` and `${a[0]:-d}` differ only in where the value
//! came from and, for the assigning forms, where it goes back to. That is what [`Target`] carries.

use super::{Target, pattern};
use crate::ast::{ParamExpansion, ReplaceScope, Word};
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::expand::arithmetic::eval_arithmetic;
use crate::expand::word::expand_word_to_string;

/// The single string a `${…}` reference stands for, once its value is known.
pub(super) fn apply(
    env: &mut Environment,
    target: &Target<'_>,
    expansion_type: &ParamExpansion,
) -> Result<String> {
    let name = target.display();
    let val = target.value(env);
    let text = match expansion_type {
        ParamExpansion::Normal => val.unwrap_or_default(),

        // `${#@}` and `${#*}` count the positionals rather than measuring the string they would
        // have joined into. `${#}` never reaches here: it is `$#`, a plain reference.
        ParamExpansion::Length => match target {
            Target::Param("@" | "*") => env.get_positional().len().to_string(),
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
                    target.assign(env, &text);
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
            Some(indirect) if !super::is_param_name(&indirect) => {
                return Err(ShellError::ExpansionError(format!(
                    "{indirect}: invalid variable name"
                )));
            }
            Some(indirect) => env.get_param(&indirect).unwrap_or_default(),
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
