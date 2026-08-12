//! `show`, `export` and `import` — reading the store out, and putting it back.

use super::{Paint, fail, parse, usage};
use oslo::macros::{self, Entry, Kind};

/// The list, or one entry in full.
pub(super) fn show(args: &[String]) -> i32 {
    let asked = match parse(args) {
        Ok(asked) => asked,
        Err(problem) => return usage(&problem),
    };
    let store = match macros::open() {
        Ok(store) => store,
        Err(problem) => return fail(&problem),
    };
    let entries = macros::all(&store);

    // A name asked for prints in full: `show gco` is somebody asking what it *is*, and flattening
    // the answer to one line would be answering a different question.
    if let Some(name) = asked.words.first() {
        let mine: Vec<&Entry> = entries.iter().filter(|e| &e.name == name).collect();
        if mine.is_empty() {
            return fail(&format!("nothing called {name}"));
        }
        for entry in mine {
            println!("{} {}", entry.kind.word(), entry.name);
            for line in entry.body.lines() {
                println!("    {line}");
            }
        }
        return 0;
    }

    if entries.is_empty() {
        println!("nothing stored yet — `oslo macros add NAME BODY`");
        return 0;
    }

    let paint = Paint::detect();
    let configured = configured_names();

    // **On a terminal it is the list, narrowed as you type** — the same widget the history finder
    // is, rather than a page of output to read. `--plain` is the way to ask for the page, and a
    // pipe gets it without asking: `Answer::NoTerminal` falls through to the printing below.
    if !asked.plain
        && let Some(status) = picked(&store, &entries, asked.edit)
    {
        return status;
    }
    for entry in &entries {
        // **Flattened to one line each**, because a function is many lines and a list of many-line
        // entries is not a list. `show NAME` above is where the whole thing lives.
        let first = entry
            .body
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let shadow = if entry.kind == Kind::Alias && configured.contains(&entry.name) {
            // Marked rather than silent: finding this out from a list is fine, finding it out by
            // wondering why your config stopped working is not.
            paint.dim("  (shadows config.lua)")
        } else {
            String::new()
        };
        if asked.plain {
            println!(
                "{}\t{}\t{}",
                entry.kind.word(),
                entry.name,
                one_line(&entry.body)
            );
        } else {
            println!(
                "{:<7} {:<18} {}{}",
                paint.dim(entry.kind.word()),
                paint.key(&entry.name),
                trimmed(first),
                shadow
            );
        }
    }
    0
}

/// The list as a widget: pick one and see it, or edit it.
///
/// `None` when there is no terminal to ask on, which is how a pipe gets the printed list instead.
fn picked(store: &macros::Store, entries: &[Entry], edit: bool) -> Option<i32> {
    // **Flattened to one row each.** A function is many lines and a list of many-line entries is
    // not a list; the whole thing is what opening one shows.
    let rows: Vec<String> = entries
        .iter()
        .map(|entry| {
            format!(
                "{:<7} {:<18} {}",
                entry.kind.word(),
                entry.name,
                trimmed(
                    entry
                        .body
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                )
            )
        })
        .collect();

    let answer = oslo::ui::ask::filter(&oslo::ui::ask::Choice {
        header: if edit {
            "edit which?".to_string()
        } else {
            "macros".to_string()
        },
        items: rows.clone(),
        height: 15,
        ..Default::default()
    });
    let chosen = match answer {
        oslo::ui::ask::Answer::NoTerminal => return None,
        oslo::ui::ask::Answer::Cancelled => return Some(1),
        oslo::ui::ask::Answer::Given(chosen) => chosen,
    };
    // The rows are positional, so the row that came back names its entry by where it sat.
    let Some(entry) = chosen
        .first()
        .and_then(|row| rows.iter().position(|r| r == row))
        .and_then(|at| entries.get(at))
    else {
        return Some(1);
    };

    if !edit {
        println!("{} {}", entry.kind.word(), entry.name);
        for line in entry.body.lines() {
            println!("    {line}");
        }
        return Some(0);
    }
    match crate::cli::editor::edit(&entry.body, entry.kind.extension(&entry.body)) {
        Ok(None) => {
            println!("unchanged");
            Some(0)
        }
        Ok(Some(body)) => {
            let updated = Entry {
                body,
                ..entry.clone()
            };
            match macros::put_and_publish(store, &updated) {
                Ok(()) => {
                    println!("{} {}", updated.kind.word(), updated.name);
                    Some(0)
                }
                Err(problem) => Some(fail(&problem)),
            }
        }
        Err(problem) => Some(fail(&problem)),
    }
}

/// The aliases `config.lua` defines, so `show` can say which stored ones win over one.
///
/// Read from the snapshot's neighbour rather than by running Lua: this is a *label on a list*, and
/// starting an interpreter to draw one would cost more than the list.
fn configured_names() -> Vec<String> {
    let Some(dir) = oslo::macros::directory() else {
        return Vec::new();
    };
    let path = dir.join("configured.names");
    std::fs::read_to_string(path)
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn one_line(body: &str) -> String {
    body.replace('\\', "\\\\").replace('\n', "\\n")
}

fn trimmed(line: &str) -> String {
    let line = line.trim();
    if line.chars().count() <= 60 {
        return line.to_string();
    }
    let cut: String = line.chars().take(57).collect();
    format!("{cut}...")
}

// ---------------------------------------------------------------- out and back

/// The interchange format: a header line per entry, then its body indented by one tab.
///
/// Chosen so it is diffable and hand-editable, which is the whole point of being able to get the
/// database back out — see the note on `export` in the help.
pub(super) fn export(args: &[String]) -> i32 {
    let asked = match parse(args) {
        Ok(asked) => asked,
        Err(problem) => return usage(&problem),
    };
    let store = match macros::open() {
        Ok(store) => store,
        Err(problem) => return fail(&problem),
    };
    let text = write_text(&macros::all(&store));
    match asked.words.first() {
        Some(path) => match std::fs::write(path, &text) {
            Ok(()) => 0,
            Err(e) => fail(&format!("{path}: {}", oslo::error::reason(&e))),
        },
        None => {
            print!("{text}");
            0
        }
    }
}

pub(super) fn write_text(entries: &[Entry]) -> String {
    let mut text = String::new();
    for entry in entries {
        text.push_str(&format!("{} {}\n", entry.kind.word(), entry.name));
        for line in entry.body.lines() {
            text.push('\t');
            text.push_str(line);
            text.push('\n');
        }
        // A body that ended without a newline is a body that ended without a newline; the marker
        // says so rather than silently adding one back on import.
        if !entry.body.ends_with('\n') && !entry.body.is_empty() {
            text.push_str("\t\\no-newline\n");
        }
    }
    text
}

pub(super) fn read_text(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut body = String::new();
    let mut ends_with_newline = true;
    for (number, line) in text.lines().enumerate() {
        if let Some(rest) = line.strip_prefix('\t') {
            let Some(last) = entries.last_mut() else {
                return Err(format!("line {}: a body before any name", number + 1));
            };
            if rest == "\\no-newline" {
                ends_with_newline = false;
                continue;
            }
            body.push_str(rest);
            body.push('\n');
            last.body = body.clone();
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        // Finish the one before, then start a new one.
        finish(&mut entries, &mut body, &mut ends_with_newline);
        let mut words = line.split_whitespace();
        let (Some(kind), Some(name)) = (words.next(), words.next()) else {
            return Err(format!("line {}: expected `kind name`", number + 1));
        };
        let Some(kind) = Kind::named(kind) else {
            return Err(format!("line {}: {kind:?} is not a kind", number + 1));
        };
        if !macros::valid_name(name) {
            return Err(format!("line {}: {name:?} is not a name", number + 1));
        }
        entries.push(Entry {
            kind,
            name: name.to_string(),
            body: String::new(),
        });
    }
    finish(&mut entries, &mut body, &mut ends_with_newline);
    Ok(entries)
}

fn finish(entries: &mut [Entry], body: &mut String, ends_with_newline: &mut bool) {
    if let Some(last) = entries.last_mut()
        && !*ends_with_newline
    {
        last.body = body.trim_end_matches('\n').to_string();
    }
    body.clear();
    *ends_with_newline = true;
}

pub(super) fn import(args: &[String]) -> i32 {
    let asked = match parse(args) {
        Ok(asked) => asked,
        Err(problem) => return usage(&problem),
    };
    let text = match asked.words.first() {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => return fail(&format!("{path}: {}", oslo::error::reason(&e))),
        },
        None => {
            let mut text = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut text) {
                return fail(&format!("stdin: {}", oslo::error::reason(&e)));
            }
            text
        }
    };
    let entries = match read_text(&text) {
        Ok(entries) => entries,
        Err(problem) => return fail(&problem),
    };
    let store = match macros::open() {
        Ok(store) => store,
        Err(problem) => return fail(&problem),
    };
    if asked.replace {
        for entry in macros::all(&store) {
            macros::remove(&store, entry.kind, &entry.name);
        }
    }
    let mut stored = 0;
    for entry in &entries {
        if let Err(problem) = macros::put(&store, entry) {
            return fail(&problem);
        }
        stored += 1;
    }
    // One snapshot for the batch rather than one per entry: the reason `put_and_publish` and `put`
    // are separate functions.
    if let Err(problem) = macros::publish(&store) {
        return fail(&problem);
    }
    println!("imported {stored}");
    0
}
