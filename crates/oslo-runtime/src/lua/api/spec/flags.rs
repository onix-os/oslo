//! Reading a flag declaration, in either of the two shapes a config may write it.
//!
//! ```lua
//! flags = {
//!   { "-v", "--verbose", desc = "say more" },     -- the array shape
//!   ["-f, --file="] = "which file",               -- carapace's own, as a key
//!   { "--env=", desc = "which one", values = { "dev", "prod" } },
//! }
//! ```
//!
//! # The modifier suffixes are carapace's, on purpose
//!
//! `=` takes a value, `?` takes an optional one, `*` may be repeated, `&` is hidden, `!` is
//! required. Reading them is [`oslo_ui::spec::flag`], shared with the spec-file reader so that the
//! two surfaces cannot drift. There is a long form too — `takes = "value"`, `hidden = true` —
//! because a modifier character is a poor thing to have to remember.

use oslo_base::value::{Table, Value};
use oslo_ui::spec::flag::{self, Modifiers};
use oslo_ui::spec::{Arg, Nargs, OptionSpec};

/// One flag from an array entry: `{ "-v", "--verbose", desc = "…" }`.
pub fn from_entry(entry: &Table) -> Option<OptionSpec> {
    let mut names = Vec::new();
    let mut modifiers = Modifiers::default();
    let written = entry
        .sequence()
        .iter()
        .filter_map(|value| match value {
            Value::Str(text) => Some(text.to_string()),
            _ => None,
        })
        .chain(super::string(entry, "name"));
    for text in written {
        let (spellings, found) = flag::parse(&text);
        names.extend(spellings);
        modifiers.merge(found);
    }
    if names.is_empty() {
        return None;
    }
    Some(finish(names, modifiers, entry))
}

/// One flag from a map entry: `["-f, --file="] = "which file"`, or the same key against a table.
pub fn from_pair(key: &str, value: &Value) -> Option<OptionSpec> {
    let (names, modifiers) = flag::parse(key);
    if names.is_empty() {
        return None;
    }
    match value {
        Value::Str(description) => Some(OptionSpec {
            description: description.to_string(),
            takes: modifiers.takes,
            repeatable: modifiers.repeatable,
            hidden: modifiers.hidden,
            required: modifiers.required,
            names,
            ..OptionSpec::default()
        }),
        Value::Table(table) => Some(finish(names, modifiers, &table.borrow())),
        _ => None,
    }
}

/// The fields that are the same however the flag was spelled.
fn finish(names: Vec<String>, modifiers: Modifiers, entry: &Table) -> OptionSpec {
    let mut all = long_form(entry);
    all.merge(modifiers);
    OptionSpec {
        names,
        description: super::string(entry, "desc")
            .or_else(|| super::string(entry, "description"))
            .unwrap_or_default(),
        takes: all.takes,
        nargs: nargs(entry),
        repeatable: all.repeatable,
        hidden: all.hidden,
        required: all.required,
        default: super::string(entry, "default"),
        values: super::values::action(&entry.get_str("values")),
    }
}

/// `takes = "value"`, `hidden = true` — the same facts written out.
fn long_form(entry: &Table) -> Modifiers {
    Modifiers {
        takes: match super::string(entry, "takes").as_deref() {
            Some("value" | "required") => Arg::Required,
            Some("optional") => Arg::Optional,
            _ => Arg::None,
        },
        repeatable: entry.get_str("repeatable").truthy(),
        hidden: entry.get_str("hidden").truthy(),
        required: entry.get_str("required").truthy(),
    }
}

/// `nargs = 2`, or `nargs = "any"` for everything up to the next flag.
fn nargs(entry: &Table) -> Nargs {
    match entry.get_str("nargs") {
        Value::Str(text) if text.to_string() == "any" => Nargs::Any,
        value => match value.as_number().and_then(|n| n.as_int()) {
            Some(-1) => Nargs::Any,
            Some(n) if n > 1 => Nargs::Exactly(n as usize),
            _ => Nargs::One,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The long form and the suffix agree, and either alone is enough.
    #[test]
    fn a_flag_may_say_it_takes_a_value_in_words_or_in_punctuation() {
        let mut entry = Table::new();
        entry.set_str("takes", Value::str("value"));
        entry.set(Value::int(1), Value::str("--file"));
        assert_eq!(from_entry(&entry).unwrap().takes, Arg::Required);

        let mut punctuation = Table::new();
        punctuation.set(Value::int(1), Value::str("--file="));
        assert_eq!(from_entry(&punctuation).unwrap().takes, Arg::Required);
    }

    /// `{ "-f", "--file=" }` — the modifier belongs to the flag, not to the spelling it follows.
    #[test]
    fn a_modifier_on_one_spelling_reaches_the_whole_flag() {
        let mut entry = Table::new();
        entry.set(Value::int(1), Value::str("-f"));
        entry.set(Value::int(2), Value::str("--file="));
        let flag = from_entry(&entry).unwrap();
        assert_eq!(flag.names, vec!["-f", "--file"]);
        assert_eq!(flag.takes, Arg::Required);
    }

    #[test]
    fn the_map_shape_carries_its_description() {
        let flag = from_pair("-o, --optarg?", &Value::str("optarg flag")).unwrap();
        assert_eq!(flag.names, vec!["-o", "--optarg"]);
        assert_eq!(flag.description, "optarg flag");
        assert_eq!(flag.takes, Arg::Optional);
        assert!(from_pair("nothing", &Value::str("x")).is_none());
    }

    #[test]
    fn nargs_is_a_count_or_the_word_any() {
        let mut entry = Table::new();
        entry.set(Value::int(1), Value::str("--files="));
        entry.set_str("nargs", Value::str("any"));
        assert_eq!(from_entry(&entry).unwrap().nargs, Nargs::Any);
        entry.set_str("nargs", Value::int(2));
        assert_eq!(from_entry(&entry).unwrap().nargs, Nargs::Exactly(2));
    }
}
