//! Names the shell can run that `$PATH` has never heard of.
//!
//! A structured verb, a tool a config registered, a stored macro. The prompt asks four sources what
//! a word is — builtin, alias, function, `$PATH` — and none of them can answer for any of these, so
//! every one of them was painted as a command that does not exist, offered by nothing on Tab, and
//! eligible to be "corrected" into whatever `$PATH` had nearest. `ls | where 'size > 1024'` reads as
//! two mistakes and runs perfectly.
//!
//! It lives here because the shell registers these and the interface draws them, and those two
//! crates cannot see each other.

use std::collections::BTreeMap;
use std::sync::RwLock;

static NAMES: RwLock<Option<BTreeMap<String, &'static str>>> = RwLock::new(None);

/// The half that is read from disk rather than registered: stored macros, autoloaded functions.
///
/// Separate because it is *replaced* wholesale rather than added to. Both halves answer the same
/// four questions, and a caller never needs to know which one an answer came from.
static DYNAMIC: RwLock<Option<BTreeMap<String, &'static str>>> = RwLock::new(None);

/// Replace the disk-backed half.
///
/// **Wholesale, because a name can go away.** A macro that was removed has to stop being a name the
/// prompt knows, and merging would only ever add.
pub fn set_dynamic(names: Vec<(String, &'static str)>) {
    let map: BTreeMap<String, &'static str> = names.into_iter().collect();
    if let Ok(mut slot) = DYNAMIC.write() {
        *slot = Some(map);
    }
}

/// Read one of the two maps.
fn with<T>(
    slot: &RwLock<Option<BTreeMap<String, &'static str>>>,
    f: impl Fn(&BTreeMap<String, &'static str>) -> T,
    empty: T,
) -> T {
    match slot.read() {
        Ok(guard) => match guard.as_ref() {
            Some(map) => f(map),
            None => empty,
        },
        Err(_) => empty,
    }
}

/// Record that `name` runs, and what kind of thing it is.
///
/// The kind is what the dropdown puts in its badge column, so it is the word a reader needs rather
/// than an internal one: `verb` for a structured stage, `tool` for one a config registered.
pub fn add(name: &str, kind: &'static str) {
    if let Ok(mut slot) = NAMES.write() {
        slot.get_or_insert_with(BTreeMap::new)
            .insert(name.to_string(), kind);
    }
}

/// Forget `name`, for a config that withdrew a tool.
pub fn remove(name: &str) {
    if let Ok(mut slot) = NAMES.write()
        && let Some(names) = slot.as_mut()
    {
        names.remove(name);
    }
}

/// Whether this name runs.
pub fn contains(name: &str) -> bool {
    with(&NAMES, |m| m.contains_key(name), false) || with(&DYNAMIC, |m| m.contains_key(name), false)
}

/// The kind of thing `name` is, or `None` if it is not one of these.
pub fn kind_of(name: &str) -> Option<&'static str> {
    with(&NAMES, |m| m.get(name).copied(), None)
        .or_else(|| with(&DYNAMIC, |m| m.get(name).copied(), None))
}

/// Whether any of them begins with `stem` — a word still being typed rather than a wrong one.
pub fn has_prefix(stem: &str) -> bool {
    if stem.is_empty() {
        return false;
    }
    let begins = |m: &BTreeMap<String, &'static str>| m.keys().any(|name| name.starts_with(stem));
    with(&NAMES, begins, false) || with(&DYNAMIC, begins, false)
}

/// Every name and its kind, in order.
pub fn all() -> Vec<(String, &'static str)> {
    let collect = |m: &BTreeMap<String, &'static str>| {
        m.iter()
            .map(|(name, kind)| (name.clone(), *kind))
            .collect::<Vec<_>>()
    };
    let mut out = with(&NAMES, collect, Vec::new());
    // A registered name wins: a config that declared a tool means the tool, whatever a file of the
    // same name on disk would otherwise have been.
    let known: std::collections::BTreeSet<String> =
        out.iter().map(|(name, _)| name.clone()).collect();
    out.extend(
        with(&DYNAMIC, collect, Vec::new())
            .into_iter()
            .filter(|(name, _)| !known.contains(name)),
    );
    out.sort();
    out
}

/// Forget all of them, for a test that wants a known vocabulary.
pub fn clear() {
    if let Ok(mut slot) = NAMES.write() {
        *slot = None;
    }
    if let Ok(mut slot) = DYNAMIC.write() {
        *slot = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name that runs is known, prefixed, kinded and listed — the four questions the prompt asks.
    #[test]
    fn a_registered_name_answers_every_question() {
        clear();
        add("where", "verb");
        add("group-by", "verb");

        assert!(contains("where"));
        assert!(!contains("wher"));
        assert!(has_prefix("wher"), "a word on its way is not a mistake");
        assert!(has_prefix("group-"));
        assert!(!has_prefix("zzz"));
        assert_eq!(kind_of("where"), Some("verb"));
        assert_eq!(kind_of("nope"), None);
        assert_eq!(
            all(),
            vec![("group-by".to_string(), "verb"), ("where".into(), "verb")]
        );

        remove("where");
        assert!(!contains("where"));
        clear();
    }

    /// An empty vocabulary answers no, rather than panicking or claiming everything.
    #[test]
    fn an_empty_vocabulary_is_quiet() {
        clear();
        assert!(!contains("where"));
        assert!(!has_prefix("w"));
        assert_eq!(kind_of("where"), None);
        assert!(all().is_empty());
    }
}
