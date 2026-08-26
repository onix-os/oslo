//! What one argc declaration becomes.
//!
//! ```text
//!   # @option -f --file <FILE>          →  flags:  "-f, --file=": "…"
//!   # @flag   -v --verbose*             →  flags:  "-v, --verbose*": "…"
//!   # @option --format[json|yaml]       →  completion.flag.format: [json, yaml]
//!   # @arg    path*[`_choice_path`]     →  dropped: a spec holds no functions (see main.rs)
//!   # @cmd / # @alias                   →  commands[] / aliases[]
//! ```
//!
//! The two models line up better than they have any right to, because both were built for the same
//! job. Where they part company it is in one direction only: argc knows things carapace has no slot
//! for — a notation (`<FILE>` is a *name* for the value), an environment variable a flag falls back
//! to, whether a positional is required. Those are counted in [`Report`] and reported, not dropped
//! in silence.

use argc::{ChoiceValue, CommandValue, FlagOptionValue};

/// What a conversion carried and what it could not.
#[derive(Default)]
pub struct Report {
    pub flags: usize,
    pub subcommands: usize,
    pub static_choices: usize,
    pub dropped_choices: usize,
    pub notations: usize,
    pub envs: usize,
}

/// A carapace spec, as much of one as this converter produces.
#[derive(Default)]
pub struct Spec {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    /// Key, description, and how many words its argument is when that is not one.
    pub flags: Vec<(String, String, Option<i64>)>,
    pub persistent: Vec<(String, String, Option<i64>)>,
    pub flag_values: Vec<(String, Vec<String>)>,
    pub positional: Vec<Vec<String>>,
    pub positional_any: Vec<String>,
    pub commands: Vec<Spec>,
}

/// One command and everything under it.
pub fn command(value: &CommandValue, report: &mut Report) -> Spec {
    let mut spec = Spec {
        name: value.name.clone(),
        aliases: value.aliases.clone(),
        description: one_line(&value.describe),
        ..Spec::default()
    };

    for flag in &value.flag_options {
        // argc's help flag is generated rather than declared, and every command has one. Offering
        // it is right; offering it as though the script had asked for it is not the point — it is
        // kept because a user typing `--h` wants it.
        let Some((key, nargs)) = flag_key(flag) else {
            continue;
        };
        let into = match flag.inherited {
            true => &mut spec.persistent,
            false => &mut spec.flags,
        };
        into.push((key, one_line(&flag.describe), nargs));
        report.flags += 1;

        if !flag.notations.is_empty() {
            report.notations += flag.notations.len();
        }
        if flag.env.is_some() {
            report.envs += 1;
        }
        if let Some(values) = choice(flag.choice.as_ref(), report) {
            // carapace keys a flag's values by its longhand, without the dashes.
            let name = flag.long_name.trim_start_matches('-');
            let name = match name.is_empty() {
                true => flag
                    .short_name
                    .as_deref()
                    .unwrap_or("")
                    .trim_start_matches('-'),
                false => name,
            };
            if !name.is_empty() {
                spec.flag_values.push((name.to_string(), values));
            }
        }
    }

    for positional in &value.positionals {
        let values = choice(positional.choice.as_ref(), report);
        if !positional.notation.is_empty() {
            report.notations += 1;
        }
        match positional.multiple {
            // `path*` answers for this position and every one after it.
            true => {
                spec.positional_any = values.unwrap_or_default();
                break;
            }
            false => spec.positional.push(values.unwrap_or_default()),
        }
    }
    // A trailing position nobody could say anything about is not a position worth declaring: an
    // empty declaration would suppress oslo's own path completion, which is the better answer.
    while spec
        .positional
        .last()
        .is_some_and(|values| values.is_empty())
    {
        spec.positional.pop();
    }

    for sub in &value.subcommands {
        report.subcommands += 1;
        spec.commands.push(command(sub, report));
    }

    spec
}

/// The carapace key for one argc flag: every spelling, then the modifiers.
///
/// Answers the `nargs` alongside it, because "takes a value" and "takes *how many*" are two facts
/// and `=` only carries the first. A flag taking two words, or every word up to the next flag,
/// spelled as though it took one leaves the walk counting the rest as the command's own arguments.
fn flag_key(flag: &FlagOptionValue) -> Option<(String, Option<i64>)> {
    let mut names: Vec<&str> = Vec::new();
    if let Some(short) = flag.short_name.as_deref().filter(|n| !n.is_empty()) {
        names.push(short);
    }
    if !flag.long_name.is_empty() && Some(flag.long_name.as_str()) != flag.short_name.as_deref() {
        names.push(&flag.long_name);
    }
    // A spelling holding a comma cannot be written: the comma separates spellings and the syntax
    // has no escape for one inside a name.
    let names: Vec<&str> = names
        .into_iter()
        .filter(|n| n.starts_with('-') && !n.contains(','))
        .collect();
    if names.is_empty() {
        return None;
    }

    let mut key = names.join(", ");
    // `flag` is argc's word for "takes no value". Everything else takes at least one.
    if !flag.flag {
        key.push('=');
    }
    // `multiple_occurs` is `-v -v -v` — the flag repeats. `multiple_values` is `--file a b c` —
    // the *argument* repeats, which is `nargs` and not a modifier.
    if flag.multiple_occurs {
        key.push('*');
    }
    if flag.required {
        key.push('!');
    }

    let nargs = match () {
        _ if flag.flag => None,
        _ if flag.multiple_values => Some(-1),
        // `# @option --pair <KEY> <VALUE>` — two notations, two words.
        _ if flag.num_args.1 > 1 => Some(flag.num_args.1 as i64),
        _ => None,
    };
    Some((key, nargs))
}

/// The values a choice offers, or `None` when it offers none this conversion can carry.
fn choice(choice: Option<&ChoiceValue>, report: &mut Report) -> Option<Vec<String>> {
    match choice? {
        ChoiceValue::Values(values) if !values.is_empty() => {
            report.static_choices += 1;
            Some(values.clone())
        }
        ChoiceValue::Values(_) => None,
        // A bash function inside the script, and data cannot hold a function. See the note at the
        // top of `main.rs` on why calling back into the script is not the answer either.
        ChoiceValue::Fn(..) => {
            report.dropped_choices += 1;
            None
        }
    }
}

/// A description as one line. argc keeps the whole comment; the dropdown has one row.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
