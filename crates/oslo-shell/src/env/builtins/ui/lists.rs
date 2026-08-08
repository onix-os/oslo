//! The widgets that show a list: `choose`, `filter`, `file` and `table`.
//!
//! Together here because they are the three that take a [`Look`](oslo_ui::ask::Look) as
//! well as a `Chrome` — every flag in [`super::look`] applies to all of them, and the option loops
//! are the same shape three times over. Keeping them side by side is what makes it obvious when
//! one of them stops accepting something the others do.

use super::{Shared, from_stdin, report, shared_flag, take};
use oslo_ui::ask::{Browse, Choice, Table, Want, choose, file, filter, parse_table, table};
use oslo_ui::matching::Fuzzy;

pub(super) fn run_choose(args: &[String], filtering: bool) -> i32 {
    let mut spec = Choice {
        filter: filtering,
        fuzzy: oslo_ui::settings::current().completion.fuzzy,
        ..Choice::default()
    };
    let mut items = Vec::new();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--header" => spec.header = take(args, &mut at),
            "--multi" | "--no-limit" => spec.multi = true,
            "--height" => spec.height = take(args, &mut at).parse().unwrap_or(spec.height).max(1),
            "--exact" => spec.fuzzy = Fuzzy::Off,
            other if other.starts_with("--") => {
                match shared_flag(&mut spec.chrome, &mut spec.look, args, &mut at) {
                    Shared::Took => {
                        at += 1;
                        continue;
                    }
                    Shared::Bad(status) => return status,
                    Shared::NotMine => {}
                }
                eprintln!("oslo: ui: {other}: unknown option");
                return 2;
            }
            other => items.push(other.to_string()),
        }
        at += 1;
    }
    // Operands win; stdin is the fallback, so `ls | ui choose` and `ui choose a b` both work.
    spec.items = if items.is_empty() {
        from_stdin()
    } else {
        items
    };
    report(if filtering {
        filter(&spec)
    } else {
        choose(&spec)
    })
}

pub(super) fn run_file(args: &[String]) -> i32 {
    let mut spec = Browse::default();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--all" | "--hidden" => spec.hidden = true,
            "--directory" | "--dir" => spec.want = Want::Directories,
            "--both" => spec.want = Want::Both,
            "--height" => spec.height = take(args, &mut at).parse().unwrap_or(spec.height).max(1),
            other if other.starts_with("--") => {
                match shared_flag(&mut spec.chrome, &mut spec.look, args, &mut at) {
                    Shared::Took => {
                        at += 1;
                        continue;
                    }
                    Shared::Bad(status) => return status,
                    Shared::NotMine => {}
                }
                eprintln!("oslo: ui file: {other}: unknown option");
                return 2;
            }
            other => spec.start = std::path::PathBuf::from(other),
        }
        at += 1;
    }
    report(file(&spec).map(|path| vec![path]))
}

pub(super) fn run_table(args: &[String]) -> i32 {
    let mut separator = ',';
    let mut header_row = false;
    let mut spec = Table::default();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--separator" | "-s" => separator = take(args, &mut at).chars().next().unwrap_or(','),
            "--header-row" | "--headers" => header_row = true,
            "--height" => spec.height = take(args, &mut at).parse().unwrap_or(spec.height).max(1),
            "--no-filter" => spec.filter = false,
            other => {
                match shared_flag(&mut spec.chrome, &mut spec.look, args, &mut at) {
                    Shared::Took => {
                        at += 1;
                        continue;
                    }
                    Shared::Bad(status) => return status,
                    Shared::NotMine => {}
                }
                eprintln!("oslo: ui table: {other}: unknown option");
                return 2;
            }
        }
        at += 1;
    }
    let (mut rows, mut raw) = parse_table(&from_stdin_raw(), separator);
    if header_row && !rows.is_empty() {
        spec.headers = rows.remove(0);
        raw.remove(0);
    }
    spec.rows = rows;
    spec.raw = raw;
    spec.fuzzy = oslo_ui::settings::current().completion.fuzzy;
    report(table(&spec).map(|row| vec![row]))
}

/// Everything on stdin, newlines and all — for the widgets that take a document rather than items.
pub(super) fn from_stdin_raw() -> String {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return String::new();
    }
    let mut text = String::new();
    let _ = std::io::stdin().lock().read_to_string(&mut text);
    text
}
