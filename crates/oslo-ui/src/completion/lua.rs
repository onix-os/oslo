//! What a **Lua** prompt completes, which is not what a shell prompt completes.
//!
//! A shell line is a command and its arguments; a Lua line is an expression. Almost none of the
//! shell's answers mean anything here — `$HOME` is not a variable in Lua, it is a lexer error, and
//! a name on `$PATH` is not something a Lua line can call. Until this module existed the Lua prompt
//! offered exactly those and nothing else: `$HO`+Tab helpfully produced `$HOME`, which cannot
//! parse, while `pri`+Tab and `oslo.`+Tab offered nothing at all.
//!
//! # The split with the runtime
//!
//! Working out *what is being typed* is text, and lives here. Working out *what names exist* needs
//! the Lua state, which this crate cannot reach — so it arrives through [`set_name_source`], the
//! same shape [`super::set_command_completer`] already uses and for the same layering reason.
//!
//! The question asked of the runtime is deliberately small: **given a path of table names, what
//! keys does it hold?** `oslo.math.` asks for `["oslo", "math"]` and gets that table's keys; a bare
//! word asks for `[]` and gets the globals. Every judgement about dots, colons, strings and word
//! boundaries stays on this side, where it can be tested without a Lua state.

use crate::dropdown::CompletionCandidate;

/// Answers the keys of the table reached by a path of names. `[]` means the globals.
///
/// The `bool` in each answer says whether the value is callable, so `print` can be offered as
/// `print(` and a plain table cannot.
pub type NameSource = std::rc::Rc<dyn Fn(&[String]) -> Vec<(String, bool)>>;

thread_local! {
    /// Thread-local for the same reason the command completer is: this calls Lua, which is not
    /// `Send`, and only the editor's thread completes anything.
    static NAMES: std::cell::RefCell<Option<NameSource>> = const { std::cell::RefCell::new(None) };
}

/// The global names in the session, each with whether it is callable.
///
/// The highlighter's half of the same source: completion asks for the keys of one table, and painting a
/// line asks for the globals so a name that exists can be drawn as what it is. Empty when no source
/// is installed, which is every non-interactive session — and then names are simply left plain.
pub fn global_names() -> Vec<(String, bool)> {
    match NAMES.with(|slot| slot.borrow().clone()) {
        Some(source) => source(&[]),
        None => Vec::new(),
    }
}

/// Install the source of Lua names. `None` removes it.
pub fn set_name_source(source: Option<NameSource>) {
    NAMES.with(|slot| *slot.borrow_mut() = source);
}

/// Lua's reserved words, which complete like names but are not in any table.
///
/// `end` earns its place on its own: it is the most-typed word in the language and the one whose
/// absence is a syntax error pages away from where it was needed.
const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// What is being completed at `pos`, once the line has been read backwards from there.
#[derive(Debug, PartialEq)]
pub struct Typed {
    /// The table path to look names up in. Empty means the globals.
    pub path: Vec<String>,
    /// The partial name being typed, which may be empty right after a dot.
    pub stem: String,
    /// Where `stem` starts, so the caller knows what to replace.
    pub at: usize,
    /// True after a `:`, where only a method makes sense and the call takes an implicit self.
    pub method: bool,
}

/// Read backwards from `pos` to find the name being typed and the table it belongs to.
///
/// Answers `None` when the cursor is somewhere no name can go — inside a string or a comment —
/// because completing there would insert code into text.
pub fn typed_at(line: &str, pos: usize) -> Option<Typed> {
    if inside_text(line, pos) {
        return None;
    }
    let bytes = line.as_bytes();
    let mut start = pos;
    while start > 0 && is_name_byte(bytes[start - 1]) {
        start -= 1;
    }
    let stem = line[start..pos].to_string();

    // A name cannot begin with a digit, so `2x` is a number followed by something rather than a
    // name being typed. Completing it would offer names that could never be written there.
    if stem.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return None;
    }

    let mut path = Vec::new();
    let mut method = false;
    let mut at = start;
    // Walk back over `a.b.c` and `a:b`, collecting the names in front of the stem. Only the first
    // separator can be a colon — `a:b:c` is not callable Lua — so anything after one stops the
    // walk rather than being silently accepted.
    loop {
        let before = line[..at].trim_end();
        let Some(sep) = before.as_bytes().last() else {
            break;
        };
        if *sep != b'.' && *sep != b':' {
            break;
        }
        if *sep == b':' {
            if !path.is_empty() {
                return None;
            }
            method = true;
        }
        let cut = before.len() - 1;
        let owner = line[..cut].trim_end();
        let mut owner_start = owner.len();
        while owner_start > 0 && is_name_byte(owner.as_bytes()[owner_start - 1]) {
            owner_start -= 1;
        }
        if owner_start == owner.len() {
            return None;
        }
        path.insert(0, owner[owner_start..].to_string());
        at = owner_start;
    }

    Some(Typed {
        path,
        stem,
        at: start,
        method,
    })
}

/// Whether `pos` sits inside a string or a comment, where a name is not a name.
///
/// A single scan forward, because Lua's strings and comments are not regular enough to read
/// backwards: `--` opens a comment only outside a string, and a `"` closes one only when it is not
/// escaped. Long brackets (`[[…]]`) are treated as strings from their opening, which is all this
/// needs — the answer only has to be right about whether a *name* may be completed here.
fn inside_text(line: &str, pos: usize) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < pos.min(bytes.len()) {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-') => return true,
            quote @ (b'"' | b'\'') => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        break;
                    }
                    i += 1;
                }
                // Ran off the end still inside the string, so the cursor is in it.
                if i >= bytes.len() {
                    return true;
                }
                if i >= pos {
                    return true;
                }
            }
            b'[' if bytes.get(i + 1) == Some(&b'[') => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Every candidate for what is being typed at `pos`, and the offset to replace from.
///
/// Answers `None` when this is not a place a Lua name can go, so the caller can decline rather
/// than offer the shell's answers instead.
pub fn candidates(line: &str, pos: usize) -> Option<(usize, Vec<CompletionCandidate>)> {
    let typed = typed_at(line, pos)?;
    let source = NAMES.with(|slot| slot.borrow().clone());
    let mut out = Vec::new();

    // Names from the Lua state itself: the globals, or the keys of the table being indexed.
    // **The same matching the shell prompt gets.** `oslo.completion.fuzzy` used not to reach here
    // at all, so `mth` offered nothing where `math` would have — a Lua prompt was the one place in
    // the shell where a prefix was the only thing that could match.
    let settings = crate::settings::current();
    let matcher = super::matchers(settings.completion.fuzzy);
    if let Some(source) = source {
        let names: Vec<(String, bool)> = source(&typed.path);
        // Each way of matching in turn, stopping at the first that finds anything, so an exact
        // match never arrives diluted with looser ones.
        let matched: Vec<&(String, bool)> = matcher
            .iter()
            .map(|how| {
                names
                    .iter()
                    .filter(|(name, _)| how.matches(name, &typed.stem))
                    .collect::<Vec<_>>()
            })
            .find(|found| !found.is_empty())
            .unwrap_or_default();
        for (name, callable) in matched {
            let (name, callable) = (name.clone(), *callable);
            // A method is reached with `:` and called, so it is never offered as a bare table.
            if typed.method && !callable {
                continue;
            }
            out.push(CompletionCandidate {
                replacement: name.clone(),
                display: name,
                description: None,
                kind: Some(if callable { "function" } else { "field" }.to_string()),
                path: None,
                detail: None,
            });
        }
    }

    // Keywords, but only where one could be written: never after a dot or a colon, which is what
    // `local` being offered as a field of `oslo` would have amounted to.
    if typed.path.is_empty() && !typed.method {
        for keyword in KEYWORDS {
            if keyword.starts_with(&typed.stem) && !typed.stem.is_empty() {
                out.push(CompletionCandidate {
                    replacement: (*keyword).to_string(),
                    display: (*keyword).to_string(),
                    description: None,
                    kind: Some("keyword".to_string()),
                    path: None,
                    detail: None,
                });
            }
        }
    }

    // `oslo.completion.lua_sources`: drop the kinds the config did not ask for, by the name each
    // candidate already carries. Separate from the shell's list because the two share no kind.
    if let Some(wanted) = &settings.completion.lua_sources {
        out.retain(|c| {
            c.kind
                .as_deref()
                .is_some_and(|kind| wanted.iter().any(|w| w == kind))
        });
    }
    out.sort_by(|a, b| a.display.cmp(&b.display));
    out.dedup_by(|a, b| a.display == b.display);
    Some((typed.at, out))
}

#[cfg(test)]
#[path = "lua/tests.rs"]
mod tests;
