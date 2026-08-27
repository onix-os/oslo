//! Reading a flag written the way carapace writes one.
//!
//! ```text
//!   -o, --optarg?
//!   │   │        └── modifiers: = takes a value, ? optional, * repeatable, & hidden, ! required
//!   │   └── the long spelling
//!   └── the short one
//! ```
//!
//! One parser, because three surfaces write the same thing: a `.yaml` spec file, a Lua config's
//! `["-f, --file="] = "…"`, and a generator's output. Two of them reading it slightly differently
//! is the kind of difference nobody notices until a flag stops taking its argument.

use super::Arg;

/// What the suffix characters said.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers {
    pub takes: Arg,
    pub repeatable: bool,
    pub hidden: bool,
    pub required: bool,
}

impl Modifiers {
    /// Fold another declaration's modifiers in. Used where a flag is written one spelling per
    /// entry, so `{ "-f", "--file=" }` takes a value.
    pub fn merge(&mut self, other: Modifiers) {
        if other.takes != Arg::None {
            self.takes = other.takes;
        }
        self.repeatable |= other.repeatable;
        self.hidden |= other.hidden;
        self.required |= other.required;
    }
}

/// Split a declaration into its spellings and its modifiers.
///
/// The modifiers sit at the end and belong to the flag, not to the spelling they happen to follow:
/// `-f, --file=` is one flag with two names, and it takes a value under either.
pub fn parse(text: &str) -> (Vec<String>, Modifiers) {
    let names = text.trim_end_matches(['=', '*', '?', '&', '!']);
    let suffix = &text[names.len()..];
    let modifiers = Modifiers {
        takes: match () {
            // `?` outranks `=`, because `--optarg?=` is written by nobody and an optional argument
            // is the weaker claim of the two.
            _ if suffix.contains('?') => Arg::Optional,
            _ if suffix.contains('=') => Arg::Required,
            _ => Arg::None,
        },
        repeatable: suffix.contains('*'),
        hidden: suffix.contains('&'),
        required: suffix.contains('!'),
    };
    let names = names
        .split(',')
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty() && name.starts_with('-'))
        .collect();
    (names, modifiers)
}

/// The name a `completion.flag` key uses: the longhand if there is one, else the shorthand, in
/// both cases without its dashes.
///
/// carapace keys flag completions by that name rather than by the spelling, so `-f, --file=` is
/// answered for under `file`.
pub fn key(names: &[String]) -> Option<String> {
    let long = names.iter().find(|name| name.starts_with("--"));
    let name = long.or_else(|| names.first())?;
    Some(name.trim_start_matches('-').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_modifiers_are_read_off_the_end() {
        assert_eq!(
            parse("--verbose"),
            (vec!["--verbose".into()], Modifiers::default())
        );
        let (names, modifiers) = parse("-o, --optarg?");
        assert_eq!(names, vec!["-o", "--optarg"]);
        assert_eq!(modifiers.takes, Arg::Optional);
        assert_eq!(parse("-v=").1.takes, Arg::Required);
    }

    #[test]
    fn several_modifiers_stack() {
        let modifiers = parse("--repeat*&!").1;
        assert!(modifiers.repeatable && modifiers.hidden && modifiers.required);
    }

    /// A word with no dash is not a spelling. Without this, `nargs: 2` written as a flag key would
    /// register a flag called `nargs`.
    #[test]
    fn a_name_without_a_dash_is_not_a_flag() {
        assert!(parse("nargs").0.is_empty());
        assert!(parse("=*").0.is_empty());
    }

    /// The longhand is what `completion.flag` keys on, dashes removed.
    #[test]
    fn the_completion_key_is_the_longhand() {
        assert_eq!(key(&["-f".into(), "--file".into()]), Some("file".into()));
        assert_eq!(key(&["-e".into()]), Some("e".into()));
        assert_eq!(key(&[]), None);
    }
}
