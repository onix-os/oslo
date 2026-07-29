//! Expanding `${a[…]}`.
//!
//! The whole point of the field-list representation is reused here rather than reinvented:
//! `"${a[@]}"` is *one [`Field`] per element*, and **no field at all** when the array is empty —
//! byte for byte the rule that makes `"$@"` with no positionals vanish instead of becoming one
//! empty argument. `cmd "${paths[@]}"` with an empty `paths` therefore runs `cmd`, not `cmd ""`.
//!
//! `[*]` is the other half of the same pair: one field, elements joined by the first character of
//! IFS, exactly as `$*` joins the positionals.
//!
//! Forms that are **not** implemented error instead of guessing. `${a[@]:1:2}` (slicing a whole
//! array) and `${a[@]#pat}` (applying an operator element-wise) are real bash, and answering them
//! with the first element or with an empty string would be the quiet wrong answer this shell is
//! being audited for.

use super::{Target, operators, origin_of};
use crate::ast::{ParamExpansion, Subscript, Word};
use crate::env::Environment;
use crate::env::scope::{ShellArray, is_valid_identifier};
use crate::error::{Result, ShellError};
use crate::expand::arithmetic::eval_arithmetic;
use crate::expand::word::{Field, Origin, Run, expand_word_to_string};
use std::borrow::Cow;

/// Expand `${name[subscript]<op>}` into the fields it contributes.
pub fn expand_array_ref(
    env: &mut Environment,
    name: &str,
    subscript: &Subscript,
    expansion_type: &ParamExpansion,
    in_quotes: bool,
) -> Result<Vec<Field>> {
    // Only a *variable* can be subscripted. `${1[@]}` and `${@[0]}` are bad substitutions in
    // bash, and answering them with an empty string would invent an array nothing ever created.
    if !is_valid_identifier(name) {
        return Err(ShellError::ExpansionError(format!(
            "${{{name}[…]}}: bad substitution"
        )));
    }
    let origin = origin_of(in_quotes);

    match subscript {
        Subscript::All | Subscript::Joined => {
            whole_array(env, name, subscript, expansion_type, origin)
        }
        Subscript::Index(word) => {
            let index = eval_index(env, name, word)?;
            check_nounset(env, name, index, expansion_type)?;
            let text = operators::apply(env, &Target::Element { name, index }, expansion_type)?;
            Ok(vec![vec![Run::new(text, origin)]])
        }
    }
}

/// `${a[@]}` and `${a[*]}`, with the handful of operators that mean something for a whole array.
fn whole_array(
    env: &Environment,
    name: &str,
    subscript: &Subscript,
    expansion_type: &ParamExpansion,
    origin: Origin,
) -> Result<Vec<Field>> {
    // A plain scalar is a one-element array as far as `[@]` is concerned: `v=x` makes
    // `"${v[@]}"` one field holding `x` and `${#v[@]}` the number 1, which is the same identity
    // that already makes `${v[0]}` answer `x`. An unset name is the empty array.
    let array = match env.get_array(name) {
        Some(array) => Cow::Borrowed(array),
        None => Cow::Owned(match env.get_var(name) {
            Some(value) => ShellArray::from_values([value]),
            None => ShellArray::default(),
        }),
    };
    let separate = matches!(subscript, Subscript::All);

    let values: Vec<String> = match expansion_type {
        ParamExpansion::Normal => array.values().map(str::to_string).collect(),

        // `${#a[@]}` is the number of elements, never the width of the joined text — and it is a
        // single field whichever subscript asked for it.
        ParamExpansion::Length => {
            return Ok(vec![vec![Run::new(array.len().to_string(), origin)]]);
        }

        // `${!a[@]}` lists the indices *in use*. A sparse array is why this is not `0..len`.
        ParamExpansion::Indirect => array.indices().map(|i| i.to_string()).collect(),

        // Everything else is an operator bash applies element-wise (`${a[@]#pat}`) or to a slice
        // of the array (`${a[@]:1:2}`). Both are real; neither is implemented, and either answered
        // with a plausible-looking string would be worse than saying so.
        _ => {
            return Err(ShellError::ExpansionError(format!(
                "${{{name}[{}]…}}: operators on a whole array are not supported yet",
                if separate { "@" } else { "*" }
            )));
        }
    };

    if separate {
        // One field per element, and none at all when there are none: the `"$@"` rule.
        return Ok(values
            .into_iter()
            .map(|v| vec![Run::new(v, origin)])
            .collect());
    }
    Ok(vec![vec![Run::new(
        values.join(&env.ifs_separator()),
        origin,
    )]])
}

/// Evaluate a subscript as arithmetic, which is what bash does for an indexed array: `${a[i+1]}`
/// reads `i`, and `${a[x]}` with `x` unset reads element 0.
fn eval_index(env: &mut Environment, name: &str, word: &Word) -> Result<i64> {
    let text = expand_word_to_string(env, word)?;
    let text = text.trim();
    if text.is_empty() {
        return Err(ShellError::ExpansionError(format!(
            "{name}: bad array subscript"
        )));
    }
    eval_arithmetic(env, text)
}

/// `set -u` applies to an element that was never assigned, exactly as it does to a plain variable.
///
/// The operators that exist to *define* what an absent value means are exempt, for the reason they
/// are exempt for scalars: `${a[3]-default}` is how a script reads a possibly-absent element under
/// `set -u`, and erroring on it would leave no way to do so.
fn check_nounset(
    env: &Environment,
    name: &str,
    index: i64,
    expansion_type: &ParamExpansion,
) -> Result<()> {
    if !env.nounset() {
        return Ok(());
    }
    if matches!(
        expansion_type,
        ParamExpansion::DefaultValue { .. }
            | ParamExpansion::UseAlternative { .. }
            | ParamExpansion::ErrorIfUnset { .. }
    ) {
        return Ok(());
    }
    if env.get_array(name).and_then(|a| a.get(index)).is_some() {
        return Ok(());
    }
    Err(ShellError::ExpansionError(format!(
        "{name}[{index}]: unbound variable"
    )))
}

#[cfg(test)]
mod tests {
    use super::expand_array_ref;
    use crate::ast::{ParamExpansion, Subscript, Word};
    use crate::env::Environment;
    use crate::env::scope::ShellArray;
    use crate::expand::word::field_text;

    /// Expand `${name[sub]<op>}` over an array of `values`, as a list of field texts.
    fn fields(values: &[&str], sub: Subscript, op: ParamExpansion) -> Result<Vec<String>, String> {
        let mut env = Environment::new();
        env.set_array("rush_arr", ShellArray::from_values(values.to_vec()));
        expand_array_ref(&mut env, "rush_arr", &sub, &op, true)
            .map(|fields| fields.iter().map(|f| field_text(f)).collect())
            .map_err(|e| e.to_string())
    }

    fn index(n: &str) -> Subscript {
        Subscript::Index(Word::from_literal(n))
    }

    /// The R2.2 rule, applied to arrays: one field per element, and none when there are none.
    #[test]
    fn at_is_one_field_per_element() {
        let got = fields(&["a b", "c"], Subscript::All, ParamExpansion::Normal);
        assert_eq!(got, Ok(vec!["a b".to_string(), "c".to_string()]));
        assert_eq!(
            fields(&[], Subscript::All, ParamExpansion::Normal),
            Ok(vec![])
        );
    }

    /// `[*]` is one field however many elements there are — including none.
    #[test]
    fn star_is_always_one_field() {
        let got = fields(&["a", "b"], Subscript::Joined, ParamExpansion::Normal);
        assert_eq!(got, Ok(vec!["a b".to_string()]));
        let got = fields(&[], Subscript::Joined, ParamExpansion::Normal);
        assert_eq!(got, Ok(vec![String::new()]));
    }

    #[test]
    fn length_counts_elements_and_indices_list_them() {
        let got = fields(&["a", "b", "c"], Subscript::All, ParamExpansion::Length);
        assert_eq!(got, Ok(vec!["3".to_string()]));
        let got = fields(&["a", "b"], Subscript::All, ParamExpansion::Indirect);
        assert_eq!(got, Ok(vec!["0".to_string(), "1".to_string()]));
    }

    /// A sparse array is the reason `${!a[@]}` cannot be `0..len`.
    #[test]
    fn indices_report_the_holes() {
        let mut env = Environment::new();
        let mut array = ShellArray::from_values(["a", "b", "c"]);
        array.remove(1);
        env.set_array("rush_sparse", array);
        let got = expand_array_ref(
            &mut env,
            "rush_sparse",
            &Subscript::All,
            &ParamExpansion::Indirect,
            true,
        )
        .unwrap();
        let texts: Vec<String> = got.iter().map(|f| field_text(f)).collect();
        assert_eq!(texts, vec!["0".to_string(), "2".to_string()]);
        assert_eq!(
            fields(&[], Subscript::All, ParamExpansion::Length),
            Ok(vec!["0".to_string()])
        );
    }

    /// A subscript is arithmetic, so `${a[i+1]}` reads `i` rather than a variable called `i+1`.
    #[test]
    fn a_subscript_is_evaluated_as_arithmetic() {
        let mut env = Environment::new();
        env.set_array("rush_ar2", ShellArray::from_values(["x", "y", "z"]));
        env.set_var("rush_i", "1", false);
        let sub = Subscript::Index(Word::from_literal("rush_i+1"));
        let got =
            expand_array_ref(&mut env, "rush_ar2", &sub, &ParamExpansion::Normal, true).unwrap();
        assert_eq!(field_text(&got[0]), "z");
    }

    #[test]
    fn an_operator_applies_to_the_selected_element() {
        let got = fields(&["a.c", "b.c"], index("1"), ParamExpansion::Length);
        assert_eq!(got, Ok(vec!["3".to_string()]));
        // An absent element is empty, not an error.
        let got = fields(&["a"], index("9"), ParamExpansion::Normal);
        assert_eq!(got, Ok(vec![String::new()]));
    }

    /// The forms rush does not implement must say so rather than answer something plausible.
    #[test]
    fn an_unimplemented_whole_array_operator_is_an_error() {
        let op = ParamExpansion::Substring {
            offset: Word::from_literal("1"),
            length: None,
        };
        assert!(fields(&["a", "b"], Subscript::All, op).is_err());
        let op = ParamExpansion::RemovePrefix {
            pattern: Word::from_literal("a"),
            longest: false,
        };
        assert!(fields(&["a", "b"], Subscript::All, op).is_err());
    }

    #[test]
    fn an_empty_subscript_is_rejected() {
        assert!(fields(&["a"], index(""), ParamExpansion::Normal).is_err());
    }
}
