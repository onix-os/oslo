//! Pathname expansion.
//!
//! The matcher is oslo's own rather than a `glob` crate because a shell needs a dialect no such
//! crate offers. Three of the obvious defaults are wrong for POSIX:
//!
//! * `*` matches a leading `.`, so `rm *` would sweep up `.git`, `.ssh` and `.env`;
//! * `**` is always globstar, while POSIX (and bash without `shopt -s globstar`) reads it as an
//!   ordinary `*` that cannot cross a `/` — and `require_literal_separator` does *not* turn that
//!   off, because the pattern is lexed into a recursive token before the options are consulted;
//! * matches come back as filesystem-normalised paths, so `./a*` yields `a1` rather than `./a1`.
//!
//! Walking the directory tree by hand fixes all three at once: each pattern component is matched
//! against one directory's entries, and every match is spelled by appending to the literal text
//! the pattern itself used.
//!
//! The pattern dialect itself lives in the `compile` submodule, because `case`, `[[ ]]` and
//! `${v#p}` need the same one and none of them is about paths. What stays here is what only a
//! path has: the `/` that splits a pattern into components, and the leading dot a component may
//! not match blindly.

mod compile;

use crate::expand::word::{Run, field_text};
use compile::{Item, compile_items, matches_items};
use std::fs;

pub use compile::ShellPattern;

/// One `/`-separated piece of a pattern.
enum Component {
    /// No metacharacters survived compilation, so this piece names a directory entry outright.
    /// `a[b` lands here too: an unterminated class is not an error in a shell, it is text.
    Literal(String),
    Pattern(Vec<Item>),
}

/// Expand one field against the filesystem, or yield its literal text when it matches nothing.
///
/// The pattern is rebuilt run by run rather than taken from the field's flat text, because
/// quoting decides per character whether `*` is a metacharacter: `echo "a"*` must glob on the
/// trailing `*` and `echo "a*"` must not glob at all.
pub fn expand_glob(field: &[Run]) -> Vec<String> {
    let chars: Vec<(char, bool)> = field
        .iter()
        .flat_map(|run| run.text.chars().map(move |ch| (ch, run.globs())))
        .collect();

    let (components, trailing_slash) = split_components(&chars);
    if !components
        .iter()
        .any(|c| matches!(c, Component::Pattern(_)))
    {
        return vec![field_text(field)];
    }

    let mut matched = walk(&components, trailing_slash);
    if matched.is_empty() {
        return vec![field_text(field)];
    }
    // bash orders a match list; byte order is the C-locale collation of that ordering.
    matched.sort_unstable();
    matched
}

/// Cut the pattern at every `/` and compile each piece.
///
/// `/` is never a metacharacter and never matched by one, so splitting first is what makes
/// `*` stop at a directory boundary. The returned flag records a trailing `/`, which restricts
/// the last component to directories and is kept in the result.
fn split_components(chars: &[(char, bool)]) -> (Vec<Component>, bool) {
    let mut pieces: Vec<Vec<(char, bool)>> = vec![Vec::new()];
    for &(ch, globs) in chars {
        if ch == '/' {
            pieces.push(Vec::new());
        } else {
            pieces
                .last_mut()
                .expect("one piece always exists")
                .push((ch, globs));
        }
    }

    // `d*/` is `d*` restricted to directories, not a component that must match the empty name.
    let trailing_slash = pieces.len() > 1 && pieces.last().is_some_and(Vec::is_empty);
    if trailing_slash {
        pieces.pop();
    }
    (pieces.iter().map(|p| compile(p)).collect(), trailing_slash)
}

/// Compile one component, remembering whether anything in it actually globs.
fn compile(chars: &[(char, bool)]) -> Component {
    let (items, has_metacharacter) = compile_items(chars);
    if has_metacharacter {
        return Component::Pattern(items);
    }
    Component::Literal(
        items
            .iter()
            .map(|i| match i {
                Item::Ch(c) => *c,
                _ => unreachable!("a component with no metacharacter holds only characters"),
            })
            .collect(),
    )
}

/// Whether a directory entry named `name` matches a compiled component.
///
/// The leading-dot rule is a *pathname* rule and lives only here: a hidden file is matched only
/// by a pattern that spells the dot out, which is what keeps `rm *` away from `.git` and `.ssh`.
/// `case .git in *)` has no such exemption, which is why the matcher itself does not know it.
fn matches(items: &[Item], name: &str) -> bool {
    if name.starts_with('.') && items.first() != Some(&Item::Ch('.')) {
        return false;
    }
    matches_items(items, name)
}

/// Match the components against the filesystem, one directory level at a time.
///
/// The accumulator holds path *prefixes* built from the pattern's own text, which is why
/// `./a*` comes back as `./a1`: nothing ever round-trips through a normalising path type.
fn walk(components: &[Component], trailing_slash: bool) -> Vec<String> {
    let mut current = vec![String::new()];

    for (index, component) in components.iter().enumerate() {
        let last = index + 1 == components.len();
        let mut next = Vec::new();

        for base in &current {
            match component {
                Component::Literal(name) => {
                    let path = format!("{base}{name}");
                    if !last {
                        next.push(path + "/");
                    } else if exists(&path) && (!trailing_slash || is_dir(&path)) {
                        next.push(finish(path, trailing_slash));
                    }
                }
                Component::Pattern(items) => {
                    for name in read_names(base) {
                        if !matches(items, &name) {
                            continue;
                        }
                        let path = format!("{base}{name}");
                        if !last {
                            if is_dir(&path) {
                                next.push(path + "/");
                            }
                        } else if !trailing_slash || is_dir(&path) {
                            next.push(finish(path, trailing_slash));
                        }
                    }
                }
            }
        }

        current = next;
        if current.is_empty() {
            break;
        }
    }
    current
}

fn finish(path: String, trailing_slash: bool) -> String {
    if trailing_slash { path + "/" } else { path }
}

/// The entries of the directory a prefix names, or nothing when it cannot be read.
///
/// An empty prefix is the working directory. `read_dir` never yields `.` or `..`, which is
/// exactly the POSIX rule that `.*` must not expand to them.
fn read_names(base: &str) -> Vec<String> {
    let dir = if base.is_empty() { "." } else { base };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Whether the path names anything at all — a dangling symlink included, since `ls -l` can
/// still show it and a glob still matches it.
fn exists(path: &str) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn is_dir(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_dir())
}

#[cfg(test)]
mod tests {
    use super::{Component, Item, compile, expand_glob, matches};
    use crate::expand::word::{Origin, Run};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn pattern(text: &str) -> Vec<Item> {
        let chars: Vec<(char, bool)> = text.chars().map(|c| (c, true)).collect();
        match compile(&chars) {
            Component::Pattern(items) => items,
            Component::Literal(_) => Vec::new(),
        }
    }

    /// A scratch directory named after the caller, so the suite's threads cannot collide.
    fn scratch(tag: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("oslo-glob-{}-{tag}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    /// Glob an absolute pattern. Absolute so the test never depends on the process-wide working
    /// directory, which the other tests in this binary are free to change.
    fn glob_in(dir: &Path, pattern: &str) -> Vec<String> {
        let field = vec![
            Run::new(format!("{}/", dir.display()), Origin::Quoted),
            Run::new(pattern, Origin::Literal),
        ];
        let prefix = format!("{}/", dir.display());
        expand_glob(&field)
            .into_iter()
            .map(|p| p.strip_prefix(&prefix).unwrap_or(&p).to_string())
            .collect()
    }

    #[test]
    fn a_field_with_no_metacharacters_is_itself() {
        let field = vec![Run::new("plain", Origin::Literal)];
        assert_eq!(expand_glob(&field), vec!["plain"]);
    }

    /// The whole point of the provenance rework: quoting suppresses globbing per character, not
    /// per word.
    #[test]
    fn quoted_metacharacters_do_not_glob() {
        let field = vec![Run::new("a*", Origin::Quoted)];
        assert_eq!(expand_glob(&field), vec!["a*"]);
        let field = vec![Run::new("nomatch\\", Origin::Quoted)];
        assert_eq!(expand_glob(&field), vec!["nomatch\\"]);
    }

    /// An unmatched pattern expands to itself with the quotes removed, not to the pattern text
    /// the matcher was handed.
    #[test]
    fn an_unmatched_pattern_yields_the_unquoted_text() {
        let field = vec![
            Run::new("no*such", Origin::Quoted),
            Run::new("*", Origin::Literal),
        ];
        assert_eq!(expand_glob(&field), vec!["no*such*"]);
    }

    #[test]
    fn an_unterminated_class_is_literal_text() {
        let field = vec![Run::new("a[b", Origin::Literal)];
        assert_eq!(expand_glob(&field), vec!["a[b"]);
    }

    #[test]
    fn stars_and_classes_match_names() {
        assert!(matches(&pattern("a*c"), "abc"));
        assert!(matches(&pattern("a*c"), "ac"));
        assert!(!matches(&pattern("a*c"), "abd"));
        assert!(matches(&pattern("a?c"), "abc"));
        assert!(!matches(&pattern("a?c"), "ac"));
        assert!(matches(&pattern("[a-c]x"), "bx"));
        assert!(!matches(&pattern("[a-c]x"), "dx"));
        assert!(matches(&pattern("[!a-c]x"), "dx"));
        assert!(matches(&pattern("[]a]x"), "]x"));
    }

    /// A named class nests inside the bracket, so its `]` must not close the bracket early.
    #[test]
    fn posix_character_classes_are_understood() {
        assert!(matches(&pattern("f[[:digit:]]"), "f1"));
        assert!(!matches(&pattern("f[[:digit:]]"), "fx"));
        assert!(matches(&pattern("[[:upper:][:digit:]]*"), "A9"));
        assert!(!matches(&pattern("[![:digit:]]*"), "1a"));
        // An unknown class name matches nothing rather than exploding.
        assert!(!matches(&pattern("[[:nosuch:]]"), "a"));
    }

    /// R2.11: a leading dot has to be spelled out, or `rm *` reaches `.git`.
    #[test]
    fn a_leading_dot_is_never_matched_by_a_metacharacter() {
        assert!(!matches(&pattern("*"), ".hidden"));
        assert!(!matches(&pattern("?hidden"), ".hidden"));
        assert!(!matches(&pattern("[.]hidden"), ".hidden"));
        assert!(matches(&pattern(".*"), ".hidden"));
        assert!(matches(&pattern(".*hidden"), ".hidden"));
        // Only the *leading* dot is special.
        assert!(matches(&pattern("a*"), "a.b"));
    }

    /// R2.12: `**` is an ordinary `*` unless globstar is on, which POSIX has no way to turn on.
    #[test]
    fn a_double_star_is_not_globstar() {
        assert_eq!(pattern("**"), pattern("*"));
        assert_eq!(pattern("a***b"), pattern("a*b"));
    }

    #[test]
    fn dotfiles_are_only_matched_when_the_dot_is_written_out() {
        let dir = scratch("dotfiles");
        fs::write(dir.join(".hidden"), "").unwrap();
        fs::write(dir.join("visible"), "").unwrap();
        assert_eq!(glob_in(&dir, "*"), vec!["visible"]);
        assert_eq!(glob_in(&dir, ".*"), vec![".hidden"]);
    }

    /// `.` and `..` are real directory entries, and every one of these patterns would match them
    /// textually. None of them may.
    #[test]
    fn dot_and_dotdot_are_never_matched() {
        let dir = scratch("dotdot");
        fs::write(dir.join(".hidden"), "").unwrap();
        assert_eq!(glob_in(&dir, ".*"), vec![".hidden"]);
        // `.?` would match `..` textually; it must not, because `..` is not a candidate at all.
        assert_eq!(glob_in(&dir, ".?"), vec![".?"]);
    }

    /// A match is spelled the way the pattern was: no normalisation, no lost `./`.
    #[test]
    fn matches_keep_the_patterns_own_path_syntax() {
        let dir = scratch("syntax");
        fs::write(dir.join("a1"), "").unwrap();
        let field = vec![Run::new(format!("{}/./a*", dir.display()), Origin::Literal)];
        assert_eq!(expand_glob(&field), vec![format!("{}/./a1", dir.display())]);
    }

    #[test]
    fn a_star_does_not_cross_a_directory_boundary() {
        let dir = scratch("separator");
        fs::create_dir(dir.join("d")).unwrap();
        fs::write(dir.join("d/a1"), "").unwrap();
        fs::write(dir.join("top"), "").unwrap();
        // Were `*` allowed to span `/`, this would also match `d/a1`.
        assert_eq!(glob_in(&dir, "*"), vec!["d", "top"]);
        assert_eq!(glob_in(&dir, "*/*"), vec!["d/a1"]);
        // …and `**` must behave identically.
        assert_eq!(glob_in(&dir, "**/*"), vec!["d/a1"]);
    }

    #[test]
    fn a_trailing_slash_restricts_the_match_to_directories() {
        let dir = scratch("dirsuffix");
        fs::create_dir(dir.join("d1")).unwrap();
        fs::write(dir.join("d2"), "").unwrap();
        assert_eq!(glob_in(&dir, "d*/"), vec!["d1/"]);
    }

    #[test]
    fn a_literal_component_after_a_pattern_must_exist() {
        let dir = scratch("literal-tail");
        fs::create_dir(dir.join("d1")).unwrap();
        fs::create_dir(dir.join("d2")).unwrap();
        fs::write(dir.join("d1/target"), "").unwrap();
        assert_eq!(glob_in(&dir, "*/target"), vec!["d1/target"]);
    }

    #[test]
    fn matches_are_sorted() {
        let dir = scratch("order");
        for name in ["c", "a", "b"] {
            fs::write(dir.join(name), "").unwrap();
        }
        assert_eq!(glob_in(&dir, "*"), vec!["a", "b", "c"]);
    }
}
