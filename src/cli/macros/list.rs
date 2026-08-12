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

    // **On a terminal it is the manager**, the same screen the history finder is rather than a page
    // of output to read. `--plain` is how you ask for the page, and a pipe gets it without asking:
    // there is no terminal to draw on, so `open` answers `None` and the printing below runs.
    //
    // **Opened even when nothing is stored**, because the database is not the only source: Tab is
    // how you reach the aliases your configuration defines, and a screen that refused to open on an
    // empty database would be hiding the one list you can always look at.
    if !asked.plain
        && let Some(status) = screen(&store)
    {
        return status;
    }

    if entries.is_empty() {
        println!("nothing stored yet — `oslo macros add --alias NAME BODY`");
        return 0;
    }

    let paint = Paint::detect();
    let configured = configured_names();
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
            // Every field, one row, tab-separated — the form something other than a person reads,
            // so nothing is left out for looking tidy.
            println!(
                "{}\t{}\t{}\t{}\t{}",
                entry.kind.word(),
                entry.name,
                if entry.active { "on" } else { "off" },
                entry.tags.join(","),
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

/// The manager, opened on the screen.
///
/// `None` when there is no terminal to draw on, which is how a pipe gets the printed list instead.
///
/// **Reopened after every edit.** Editing means leaving the alternate screen — `nvim` wants it — so
/// the screen closes, the editor runs, and it comes back with the query it had. Trying to keep it
/// open around a child that also drives the terminal is how two programs end up fighting over the
/// same termios.
fn screen(store: &macros::Store) -> Option<i32> {
    let mut query = String::new();
    loop {
        let items = items_for(store);
        let mut backing = Database { store };
        match oslo::ui::manager::open(items, &query, &mut backing)? {
            oslo::ui::manager::Outcome::Cancelled => return Some(0),
            oslo::ui::manager::Outcome::Edit { item, query: q } => {
                query = q;
                if let Some(status) = edit_one(store, &item) {
                    return Some(status);
                }
            }
        }
    }
}

/// Everything the screen shows: the database, then what the configuration defined.
fn items_for(store: &macros::Store) -> Vec<oslo::ui::manager::Item> {
    let session = oslo::track::session::id();
    let stored = macros::all(store).into_iter().map(|entry| (entry, true));
    let elsewhere = macros::live::from_elsewhere()
        .into_iter()
        .map(|entry| (entry, false));
    stored
        .chain(elsewhere)
        .map(|(entry, from_database)| oslo::ui::manager::Item {
            kind: entry.kind.word().to_string(),
            first: summary(&entry),
            session_off: macros::live::session::is_off(&session, &entry.name),
            stored: from_database,
            name: entry.name,
            tags: entry.tags,
            created: entry.created,
            active: entry.active,
        })
        .collect()
}

/// What a row says about a macro, beside its name.
///
/// **An alias is its body; a function or a script is its language.** A row is one line and a script
/// is two hundred, so the first line of one is `#!/usr/bin/env python3` — which says nothing you
/// could not have guessed and takes the width that a name and a tag can use. What is worth knowing
/// at a glance is what you are about to open, so the row says `python3` and Enter shows the rest.
fn summary(entry: &Entry) -> String {
    match entry.kind {
        Kind::Func | Kind::Script => macros::shebang_interpreter(&entry.body)
            .unwrap_or_else(|| entry.kind.extension(&entry.body).to_string()),
        _ => trimmed(
            entry
                .body
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or(""),
        ),
    }
}

/// Open one in the editor and store what comes back. `Some` only when something went wrong enough
/// to stop, because an editor that would not open is not a reason to lose the screen.
fn edit_one(store: &macros::Store, item: &oslo::ui::manager::Item) -> Option<i32> {
    let Some(kind) = Kind::named(&item.kind) else {
        return None;
    };
    // **A configured alias is copied in before it is edited.** It came from a file this does not
    // own and cannot write back to; copying it makes it yours, which is also the only way to get
    // one that can be turned off.
    let entry = macros::get(store, kind, &item.name).unwrap_or_else(|| {
        macros::live::from_elsewhere()
            .into_iter()
            .find(|e| e.kind == kind && e.name == item.name)
            .unwrap_or_else(|| Entry::new(kind, &item.name, ""))
    });

    match crate::cli::editor::edit(&entry.body, kind.extension(&entry.body)) {
        Ok(None) => None,
        Ok(Some(body)) => {
            let updated = Entry { body, ..entry };
            if let Err(problem) = macros::put_and_publish(store, &updated) {
                return Some(fail(&problem));
            }
            None
        }
        Err(problem) => Some(fail(&problem)),
    }
}

/// The screen's hands: it decides, this does it.
struct Database<'a> {
    store: &'a macros::Store,
}

impl oslo::ui::manager::Backing for Database<'_> {
    fn act(&mut self, item: &oslo::ui::manager::Item, act: oslo::ui::manager::Act) {
        use oslo::ui::manager::Act;
        let Some(kind) = Kind::named(&item.kind) else {
            return;
        };
        match act {
            Act::Forget => {
                let _ = macros::remove_and_publish(self.store, kind, &item.name);
            }
            Act::Everywhere(active) => {
                macros::set_active(self.store, kind, &item.name, active);
                let _ = macros::publish(self.store);
            }
            // **The parent's session, not this process's.** `oslo macros` is a child of the shell
            // whose macros it is managing, and the session it means is the one that will read the
            // file — which is the shell's.
            Act::Session(off) => {
                let _ = macros::live::session::set(&oslo::track::session::id(), &item.name, off);
            }
        }
    }
}

/// The aliases `config.lua` defines, so `show` can say which stored ones win over one.
///
/// Read from the snapshot's neighbour rather than by running Lua: this is a *label on a list*, and
/// starting an interpreter to draw one would cost more than the list.
fn configured_names() -> Vec<String> {
    macros::live::from_elsewhere()
        .into_iter()
        .map(|entry| entry.name)
        .collect()
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
        let mut header = format!("{} {}", entry.kind.word(), entry.name);
        if !entry.active {
            header.push_str(" off");
        }
        for tag in &entry.tags {
            header.push_str(" #");
            header.push_str(tag);
        }
        text.push_str(&header);
        text.push('\n');
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
        // Anything after the name is a label: `#tag` for a tag, `off` for one that is turned off.
        let mut entry = Entry::new(kind, name, "");
        for word in words {
            match word.strip_prefix('#') {
                Some(tag) if macros::valid_tag(tag) => entry.tags.push(tag.to_string()),
                Some(tag) => return Err(format!("line {}: {tag:?} is not a tag", number + 1)),
                None if word == "off" => entry.active = false,
                None => return Err(format!("line {}: {word:?} means nothing here", number + 1)),
            }
        }
        entries.push(entry);
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
