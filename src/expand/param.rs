//! Parameter expansion: `$x`, `${x}`, and every `${x<op>word}` operator.
//!
//! Two rules shape this module.
//!
//! The first is that an *operand is a word, not a string*. `${x:-$HOME}` has to expand its
//! default and `${x:=$y}` has to persist the expanded text, so every payload arrives as a
//! [`crate::ast::Word`] and is expanded here, with quote removal, when the operator needs it.
//!
//! The second is that an unrecognised `${...}` body is an **error**. The lexer hands anything it
//! cannot parse through as a parameter *name*; looking that up would answer `""` for every form
//! the shell does not implement, and a wrong-but-quiet empty string is precisely how nine
//! separate unimplemented operators went unnoticed. Rejecting a body that is not a name turns
//! that fallback into a diagnostic instead.

mod array;
mod operators;
mod pattern;

use crate::ast::ParamExpansion;
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::expand::word::{Field, Origin, Run};

pub use array::expand_array_ref;

/// What an expansion reads from, and what the assigning `${x:=v}` forms write back to.
///
/// A scalar and an array element differ in exactly those two operations; every operator in
/// `operators` is written against this rather than against a variable name, so `${a[0]%.c}`
/// cannot drift away from `${v%.c}`.
pub(crate) enum Target<'a> {
    /// A parameter by name: a variable, a positional, or a special parameter.
    Param(&'a str),
    /// One element of an indexed array, with its subscript already evaluated.
    Element { name: &'a str, index: i64 },
    /// A value already in hand, with no slot in the environment behind it.
    ///
    /// This is what element-wise application needs: `${a[@]#pat}` runs the operator over values
    /// the array path already read out, and there is no per-element name to look up. Nothing can
    /// assign through it, which is why the `${a[@]:=v}` family is still refused rather than
    /// routed here.
    Value(&'a str),
}

impl Target<'_> {
    /// The value as it stands, or `None` when the parameter or element does not exist.
    fn value(&self, env: &Environment) -> Option<String> {
        match self {
            Self::Param(name) => env.get_param(name),
            Self::Element { name, index } => env
                .get_array(name)
                .and_then(|array| array.get(*index))
                .map(str::to_string),
            Self::Value(value) => Some((*value).to_string()),
        }
    }

    /// Persist `text` — what `${x:=v}` does. An element assignment writes the element, not the
    /// whole variable, so `${a[2]:=x}` leaves the rest of the array alone.
    fn assign(&self, env: &mut Environment, text: &str) {
        match self {
            Self::Param(name) => {
                env.set_var(name, text, false);
            }
            Self::Element { name, index } => {
                env.set_array_element(name, *index, text);
            }
            // `operators::map_elements` accepts only the four rewriting operators, none of which
            // assigns, so this arm is unreachable by construction.
            Self::Value(_) => unreachable!("an element-wise operator never assigns"),
        }
    }

    /// How the reference is named in a diagnostic — as the script wrote it.
    fn display(&self) -> String {
        match self {
            Self::Param(name) => (*name).to_string(),
            Self::Element { name, index } => format!("{name}[{index}]"),
            Self::Value(value) => (*value).to_string(),
        }
    }
}

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
    let origin = origin_of(in_quotes);

    // `$@` and `$*` are the only parameters that stand for a *list*, and only `$@` keeps its
    // members apart. Everything list-valued has to be answered here: falling through to the
    // scalar path below joins the positionals with a space first, which is how `"${@:2}"` came
    // to hand back one character-sliced field instead of the arguments it was forwarding.
    if matches!(name, "@" | "*") {
        if matches!(expansion_type, ParamExpansion::Normal) {
            return Ok(positional_fields(env, name, origin));
        }
        if let Some(fields) = array::positional_list(env, name, expansion_type, origin)? {
            return Ok(fields);
        }
    }

    if let Some(fields) = payload_fields(env, name, expansion_type, in_quotes)? {
        return Ok(fields);
    }

    let text = expand_to_string(env, name, expansion_type)?;
    Ok(vec![vec![Run::new(text, origin)]])
}

/// The fields a `${x-word}` or `${x+word}` payload contributes, when the payload is what wins.
///
/// These two operators are the only ones whose operand becomes the expansion's *result* rather
/// than a pattern, a subscript or a message — so the result can be more than one field.
/// `${1+"$@"}` is the reason this exists: it is the pre-POSIX way to forward arguments, still what
/// modernish's own diagnostics are written with, and collapsing it to a single string joined the
/// arguments on a space and then re-split that join on `IFS`. `printf '%s\n' ${1+"$@"}` printed
/// one line per *word* instead of one per argument.
///
/// `${x:=word}` is deliberately not here: it has to persist a single string, and routing it
/// through the fields path would leave the two halves of the operator disagreeing about what it
/// assigned.
fn payload_fields(
    env: &mut Environment,
    name: &str,
    expansion_type: &ParamExpansion,
    in_quotes: bool,
) -> Result<Option<Vec<Field>>> {
    let value = Target::Param(name).value(env);
    let payload = match expansion_type {
        ParamExpansion::UseAlternative {
            alternative,
            test_null,
        } if operators::is_present(&value, *test_null) => alternative,
        ParamExpansion::DefaultValue {
            default,
            assign_if_unset: false,
            test_null,
        } if !operators::is_present(&value, *test_null) => default,
        _ => return Ok(None),
    };

    let mut fields = crate::expand::word::expand_word_fields_in(env, payload, in_quotes)?;
    if !in_quotes {
        // Unquoted, the payload's own literal text is part of an expansion result, and POSIX
        // splits results: `IFS=:; ${x-a:b}` is two fields. Only the payload's *quoted* runs are
        // exempt, and those already carry `Origin::Quoted`.
        for run in fields.iter_mut().flatten() {
            if run.origin == Origin::Literal {
                run.origin = Origin::Expanded;
            }
        }
    }
    Ok(Some(fields))
}

/// Where an expansion's output came from, which decides whether it may be field-split.
fn origin_of(in_quotes: bool) -> Origin {
    if in_quotes {
        Origin::Quoted
    } else {
        Origin::Expanded
    }
}

/// The single string a `${name<op>}` reference stands for.
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
    operators::apply(env, &Target::Param(name), expansion_type)
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
    array::list_fields(
        env.get_positional(),
        name == "@",
        &env.ifs_separator(),
        origin,
    )
}

#[cfg(test)]
mod tests {
    use super::{expand_param, expand_to_string, is_param_name, positional_fields};
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

    /// Expand `${@<op>}` / `${*<op>}` over `params`, as a list of field texts.
    fn positional(params: &[&str], name: &str, op: ParamExpansion) -> Result<Vec<String>, String> {
        let mut env = Environment::new();
        env.set_positional(params.iter().map(|p| (*p).to_string()).collect());
        expand_param(&mut env, name, &op, true)
            .map(|fields| fields.iter().map(|f| field_text(f)).collect())
            .map_err(|e| e.to_string())
    }

    fn slice(offset: &str, length: Option<&str>) -> ParamExpansion {
        ParamExpansion::Substring {
            offset: lit(offset),
            length: length.map(lit),
        }
    }

    /// R11/B5: `${@:1}` slices the *argument list*.
    ///
    /// This used to fall through to the scalar path, which joins the positionals with a space and
    /// then cuts characters out of that string — so `set -- a b c; "${@:1}"` handed back the one
    /// field `" b c"` and every `"${@:2}"` forwarding idiom silently corrupted its arguments.
    #[test]
    fn slicing_the_positionals_selects_arguments_not_characters() {
        let args = ["a", "b", "c"];
        let got = positional(&args, "@", slice("1", None));
        assert_eq!(got, Ok(vec!["a".into(), "b".into(), "c".into()]));
        assert_eq!(
            positional(&args, "@", slice("2", None)),
            Ok(vec!["b".into(), "c".into()])
        );
        assert_eq!(
            positional(&args, "@", slice("1", Some("2"))),
            Ok(vec!["a".into(), "b".into()])
        );
        // Negative counts back from the end of the same list; past the front selects nothing, and
        // `"$@"` with nothing selected is no field rather than one empty one.
        assert_eq!(
            positional(&args, "@", slice("-2", None)),
            Ok(vec!["b".into(), "c".into()])
        );
        assert_eq!(positional(&args, "@", slice("9", None)), Ok(vec![]));
        // An argument that contains the separator stays one field, which is the whole reason the
        // joined string is the wrong thing to slice.
        assert_eq!(
            positional(&["a b", "c"], "@", slice("1", None)),
            Ok(vec!["a b".into(), "c".into()])
        );
        // `${*:1}` is the joined form of the same slice: one field, always.
        assert_eq!(
            positional(&args, "*", slice("2", None)),
            Ok(vec!["b c".into()])
        );
        assert_eq!(
            positional(&args, "*", slice("9", None)),
            Ok(vec![String::new()])
        );
        // bash makes a negative length fatal for a list, unlike for a string.
        assert!(positional(&args, "@", slice("1", Some("-1"))).is_err());
    }

    /// The list bash slices is `$0 $1 $2 …`, one longer than `$@` — which is why the forwarding
    /// idiom is written `"${@:1}"` and `"${@:0}"` picks up the shell's own name.
    #[test]
    fn a_positional_slice_is_offset_by_arg_zero() {
        let mut env = Environment::new();
        env.set_positional(vec!["a".into(), "b".into()]);
        let zero = env.get_param("0").expect("$0 is always set");
        let got = expand_param(&mut env, "@", &slice("0", None), true).unwrap();
        let texts: Vec<String> = got.iter().map(|f| field_text(f)).collect();
        assert_eq!(texts, vec![zero, "a".to_string(), "b".to_string()]);
    }

    /// The twin of the array rule: `${@#pat}` rewrites each argument, and `$0` takes no part.
    #[test]
    fn a_pattern_operator_maps_over_the_positionals() {
        let op = ParamExpansion::RemovePrefix {
            pattern: lit("a"),
            longest: false,
        };
        let got = positional(&["a.c", "b.c"], "@", op);
        assert_eq!(got, Ok(vec![".c".into(), "b.c".into()]));

        let op = ParamExpansion::CaseConvert {
            pattern: None,
            upper: true,
            all: true,
        };
        assert_eq!(positional(&["ab", "cd"], "*", op), Ok(vec!["AB CD".into()]));
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

    /// `oslo_ref`, not `name`: [`Environment::new`] inherits the real environment, and a plain
    /// word like `name` is genuinely exported by some development shells — which would make the
    /// unset case below pass or fail depending on who ran the tests.
    #[test]
    fn indirection_follows_the_named_parameter() {
        let vars = [("oslo_target", "payload"), ("oslo_ref", "oslo_target")];
        let got = expand(&vars, "oslo_ref", ParamExpansion::Indirect);
        assert_eq!(got, Ok("payload".into()));
        // Only the *inner* parameter may be unset; that is an empty string, not an error.
        let vars = [("oslo_ref", "oslo_nosuchvar")];
        let got = expand(&vars, "oslo_ref", ParamExpansion::Indirect);
        assert_eq!(got, Ok(String::new()));
        // bash aborts the expansion when the referring parameter is unset, or holds anything
        // that is not a name — including the empty string.
        for vars in [vec![], vec![("oslo_ref", "")], vec![("oslo_ref", "a b")]] {
            let got = expand(&vars, "oslo_ref", ParamExpansion::Indirect);
            assert!(got.is_err(), "{vars:?} expanded to {got:?}");
        }
    }
}
