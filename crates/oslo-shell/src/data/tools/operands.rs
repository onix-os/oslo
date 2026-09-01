//! Reading a verb's operands, and refusing the ones it cannot honour.

use super::*;

/// Refuse an operand the verb was never going to read.
///
/// **A word a verb ignores is a mistake, not a decoration.** `ls | length extra` answered as
/// though `extra` had not been typed, and `ls | first 5 10` quietly used the 5 — the same
/// silent-acceptance bug that `printf -Z` and `trap -z EXIT` had, in the tools oslo invented
/// rather than the ones it inherited. `wanted` is how many operands the verb actually reads.
pub(super) fn too_many(name: &str, words: &[String], wanted: usize) -> Option<Outcome> {
    let extra = words.get(wanted + 1)?;
    eprintln!("{}{name}: {extra}: too many arguments", origin_now());
    Some((2, None))
}

/// A count operand: `first 5`, `final 3`. Absent means one.
///
/// A word that is not a whole number used to become 1, so `first -5` and `first many` both
/// answered a single row and looked as though they had worked.
pub(super) fn count_operand(name: &str, words: &[String]) -> Result<usize, Outcome> {
    match words.get(1) {
        None => Ok(1),
        Some(word) => word.parse::<usize>().map_err(|_| {
            eprintln!("{}{name}: {word}: a count is a whole number", origin_now());
            (2, None)
        }),
    }
}

/// The flags and keys of a `sort-by`.
///
/// Short flags cluster (`-rn`) the way every shell user expects, and `--` ends them — a column
/// really could be called `-x`, and POSIX has one way of saying so.
pub(super) fn sort_operands(
    words: &[String],
) -> Result<(verbs::SortOptions, Vec<String>), Outcome> {
    let mut options = verbs::SortOptions::default();
    let mut keys = Vec::new();
    let mut flags_done = false;
    for word in words {
        if flags_done || !word.starts_with('-') || word == "-" {
            keys.push(word.clone());
            continue;
        }
        if word == "--" {
            flags_done = true;
            continue;
        }
        let long = word.strip_prefix("--");
        let ok = match long {
            Some("reverse") => set(&mut options.reverse),
            Some("natural") => set(&mut options.natural),
            Some("ignore-case") => set(&mut options.ignore_case),
            Some(_) => false,
            // A cluster: every letter has to be one this knows, or the whole word is refused.
            None => word.chars().skip(1).all(|c| match c {
                'r' => set(&mut options.reverse),
                'n' => set(&mut options.natural),
                'i' => set(&mut options.ignore_case),
                _ => false,
            }),
        };
        if !ok {
            eprintln!(
                "{}sort-by: {word}: not an option; sort-by knows -r, -n and -i",
                origin_now()
            );
            return Err((2, None));
        }
    }
    Ok((options, keys))
}

fn set(flag: &mut bool) -> bool {
    *flag = true;
    true
}

/// Refuse a column name that no row in the stream has.
///
/// **Not the same as the per-row rule.** Rows are allowed to disagree about their columns, so
/// `cols` keeps a name that only some of them carry — see [`verbs::cols`]. A name that *no* row
/// has is a different thing: it cannot be a legitimate gap, only a typo, and answering with a
/// stream of empty rows is the worst way to report one.
pub(super) fn unknown_column(
    name: &str,
    words: &[String],
    rows: &[Record],
    wanted: &[String],
) -> Option<Outcome> {
    // Nothing to check against: an empty stream says nothing about which columns exist.
    if rows.is_empty() {
        return None;
    }
    // A path counts as present when it resolves in *any* row, and an optional step (`a.b?`) is
    // present by construction — it said the absence was expected, so refusing it here would make
    // `?` mean nothing.
    let missing = wanted.iter().find(|column| {
        let path = crate::data::path::Path::parse(column);
        !rows
            .iter()
            .any(|row| matches!(path.get(row), Ok(Some(_)) | Ok(None)))
    })?;
    crate::env::complain(
        words,
        missing,
        &format!("{name}: {missing}: no such column"),
        "no column of that name",
        Some(&present(rows)),
    );
    Some((2, None))
}

/// The columns the stream does have, for the help line under the caret.
///
/// **This is the whole reason refusing beats emitting empty rows.** The shell already knows the
/// answer to the question a mistyped name is asking, and a person who typed `nmae` wants the list,
/// not a second guess at what they meant.
fn present(rows: &[Record]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for row in rows {
        for column in row.columns() {
            if !names.contains(&column.as_str()) {
                names.push(column);
            }
        }
    }
    // A wide stream has forty columns and listing them all would push the caret off the screen.
    // Twelve is enough to recognise the one you meant to type.
    let shown = names.len().min(12);
    let more = match names.len() > shown {
        true => format!(", and {} more", names.len() - shown),
        false => String::new(),
    };
    format!("the columns here are: {}{more}", names[..shown].join(", "))
}
