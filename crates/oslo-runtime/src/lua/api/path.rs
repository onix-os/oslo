//! `oslo.path` — pathname manipulation, with no filesystem involved.
//!
//! Split from [`super::fs`] because the distinction is worth keeping: nothing here touches the
//! disk, so nothing here can fail for a reason outside the caller's control. `oslo.path.parent`
//! answers for a path that does not exist, and `oslo.path.join` cannot report a permission error.
//! That is why these return plain values while `oslo.fs` returns `nil, message`.
//!
//! Paths are byte strings in the kernel and `String`s here, which is the same compromise
//! [`oslo_base::value::Value`] makes everywhere. A filename that is not valid UTF-8 is
//! rare and survives round-tripping through the shell; it does not survive this API, and that is
//! recorded rather than hidden.

use super::util::{list, put, text};
use oslo_base::value::{Table, Value};
use std::path::{Component, Path, PathBuf};

pub fn build() -> Value {
    let mut path = Table::new();

    // oslo.path.join("a", "b", "c") -> "a/b/c"
    //
    // An absolute component restarts the path, as `Path::join` and every other language's
    // `join` do: `join("/etc", "/tmp")` is `/tmp`. Surprising the first time, and relied on for
    // "a default, unless the caller gave an absolute one".
    put(&mut path, "join", |_, args| {
        let mut joined = PathBuf::new();
        for i in 1..=args.len() {
            joined.push(text(&args, i, "oslo.path.join")?);
        }
        Ok(vec![Value::str(joined.to_string_lossy())])
    });

    // oslo.path.parent("/a/b/c.txt") -> "/a/b"
    put(&mut path, "parent", |_, args| {
        let p = text(&args, 1, "oslo.path.parent")?;
        Ok(vec![match Path::new(&p).parent() {
            // A path with no parent answers nil rather than "" or ".": `/` and a bare `name` are
            // both genuinely parentless, and inventing `.` would make `parent` non-injective.
            Some(parent) if !parent.as_os_str().is_empty() => Value::str(parent.to_string_lossy()),
            _ => Value::Nil,
        }])
    });

    // oslo.path.name("/a/b/c.txt") -> "c.txt"
    put(&mut path, "name", |_, args| {
        let p = text(&args, 1, "oslo.path.name")?;
        Ok(vec![match Path::new(&p).file_name() {
            Some(name) => Value::str(name.to_string_lossy()),
            None => Value::Nil,
        }])
    });

    // oslo.path.stem("/a/b/c.txt") -> "c", oslo.path.ext(...) -> "txt"
    //
    // The extension comes back *without* the dot, because `"." .. ext` is easy to write and
    // `ext:sub(2)` is easy to forget. A dotfile has no extension: `.bashrc` is a name, not an
    // extension of an empty stem, which is what `Path` says too.
    put(&mut path, "stem", |_, args| {
        let p = text(&args, 1, "oslo.path.stem")?;
        Ok(vec![match Path::new(&p).file_stem() {
            Some(stem) => Value::str(stem.to_string_lossy()),
            None => Value::Nil,
        }])
    });
    put(&mut path, "ext", |_, args| {
        let p = text(&args, 1, "oslo.path.ext")?;
        Ok(vec![match Path::new(&p).extension() {
            Some(ext) => Value::str(ext.to_string_lossy()),
            None => Value::Nil,
        }])
    });

    put(&mut path, "is_absolute", |_, args| {
        let p = text(&args, 1, "oslo.path.is_absolute")?;
        Ok(vec![Value::Bool(Path::new(&p).is_absolute())])
    });

    // oslo.path.normalize("a/./b/../c") -> "a/c"
    //
    // Lexical only, so it never follows a symlink and never touches the disk. That makes it a
    // different answer from `oslo.fs.realpath` whenever `b` is a symlink — and the right one for
    // displaying a path, where resolving links would show the user somewhere they did not type.
    put(&mut path, "normalize", |_, args| {
        let p = text(&args, 1, "oslo.path.normalize")?;
        Ok(vec![Value::str(normalize(Path::new(&p)))])
    });

    // oslo.path.split("/a/b/c") -> {"a", "b", "c"}
    put(&mut path, "split", |_, args| {
        let p = text(&args, 1, "oslo.path.split")?;
        let parts = Path::new(&p)
            .components()
            .filter_map(|c| match c {
                Component::Normal(part) => Some(Value::str(part.to_string_lossy())),
                Component::RootDir => Some(Value::str("/")),
                Component::ParentDir => Some(Value::str("..")),
                // `.` and a prefix carry no information a caller can use.
                _ => None,
            })
            .collect::<Vec<_>>();
        Ok(vec![list(parts)])
    });

    // oslo.path.relative_to("/a/b/c", "/a") -> "b/c"
    put(&mut path, "relative_to", |_, args| {
        let p = text(&args, 1, "oslo.path.relative_to")?;
        let base = text(&args, 2, "oslo.path.relative_to")?;
        Ok(vec![match Path::new(&p).strip_prefix(&base) {
            Ok(rest) => Value::str(rest.to_string_lossy()),
            // Not an error and not the original path: nil says "that is not under this base",
            // where returning the input would quietly look like a successful answer.
            Err(_) => Value::Nil,
        }])
    });

    // oslo.path.expand("~/x") -> "/home/you/x"
    //
    // Only a leading `~`, which is where a shell expands it too. `~user` is not resolved: it
    // needs a passwd lookup, and answering the wrong home directory is worse than not answering.
    put(&mut path, "expand", |_, args| {
        let p = text(&args, 1, "oslo.path.expand")?;
        Ok(vec![Value::str(expand_tilde(&p))])
    });

    Value::table(path)
}

/// Resolve `.` and `..` textually, keeping a leading `/` and any `..` that cannot be cancelled.
///
/// A leading `..` survives because `../x` normalised to `x` would name a different file. In an
/// absolute path it *can* be dropped, since `/..` is `/`.
fn normalize(path: &Path) -> String {
    let absolute = path.is_absolute();
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => match parts.last().map(String::as_str) {
                Some("..") | None => {
                    if !absolute {
                        parts.push("..".to_string());
                    }
                }
                _ => {
                    parts.pop();
                }
            },
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
        }
    }
    let joined = parts.join("/");
    match (absolute, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        // Everything cancelled out, and a path has to name something: `.` is where you are.
        (false, true) => ".".to_string(),
        (false, false) => joined,
    }
}

/// Expand a leading `~` from `$HOME`.
pub(super) fn expand_tilde(path: &str) -> String {
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_string();
    };
    // `~user` is left alone rather than guessed at.
    if !rest.is_empty() && !rest.starts_with('/') {
        return path.to_string();
    }
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => format!("{home}{rest}"),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;
    use std::path::Path;

    #[test]
    fn normalisation_cancels_what_it_can_and_keeps_what_it_cannot() {
        assert_eq!(normalize(Path::new("a/./b/../c")), "a/c");
        assert_eq!(normalize(Path::new("/a/b/../../c")), "/c");
        // A relative `..` at the front names a real, different place and has to survive.
        assert_eq!(normalize(Path::new("../a")), "../a");
        assert_eq!(normalize(Path::new("../../a")), "../../a");
        // An absolute one does not: `/..` is `/`.
        assert_eq!(normalize(Path::new("/../a")), "/a");
        assert_eq!(normalize(Path::new("a/..")), ".");
        assert_eq!(normalize(Path::new("/")), "/");
    }
}
