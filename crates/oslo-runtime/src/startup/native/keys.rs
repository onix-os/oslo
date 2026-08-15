//! oslo's names for keys, in the spellings a config and a hook each use.
//!
//! Two functions rather than one, and the doc on each says why: a binding may only be offered a
//! name it could actually bind, while a hook is told about every key there is.

use oslo_ui::term::Key;

/// oslo's name for a key, in the spelling `oslo.keys` uses.
///
/// `None` for anything a config could not name — an ordinary character, above all, since asking
/// Lua about every letter typed would put a hash lookup on the hot path for nothing.
pub(super) fn key_name(key: Key) -> Option<String> {
    Some(match key {
        Key::Ctrl(c) => format!("ctrl-{c}"),
        Key::Alt(c) if !c.is_control() => format!("alt-{c}"),
        Key::ToggleScope => "tab".to_string(),
        Key::BackTab => "shift-tab".to_string(),
        Key::Up => "up".to_string(),
        Key::Down => "down".to_string(),
        Key::Left => "left".to_string(),
        Key::Right => "right".to_string(),
        Key::Home => "home".to_string(),
        Key::End => "end".to_string(),
        Key::PageUp => "pageup".to_string(),
        Key::PageDown => "pagedown".to_string(),
        Key::Function(number @ 1..=12) => format!("f{number}"),
        // **Space, and only space, out of `Key::Char`.** The line-editor page lists it among the
        // names a config may bind, and it had none here — so `oslo.keys["space"]` was read without
        // complaint and never fired. Every other character is still ruled out, which is what keeps
        // this off the path of simply typing.
        //
        // Enter, Esc, Backspace and Delete are deliberately *not* here; `is_key_name` refuses them
        // now rather than accepting names nothing can produce. See `hook_key_name` below, which is
        // the superset and exists for exactly that reason.
        Key::Char(' ') => "space".to_string(),
        // The chords `term` folded into shared names. A config that bound `ctrl-a` means the
        // chord, so the name has to come back out even though the key arrived as `Home`.
        Key::Clear => "ctrl-u".to_string(),
        _ => return None,
    })
}

/// oslo's name for a key, for the `key` hook — which sees *every* key.
///
/// A superset of [`key_name`], and separate from it on purpose. That one answers only for keys a
/// config could bind, because it feeds `oslo.keys` and a name there that cannot be bound would be
/// a lie. The hook has no such contract: it reports what was pressed, so Enter and Esc and an
/// ordinary letter all need a name.
///
/// The `char` half is `Some` only for an ordinary character, which is what lets a handler tell
/// `k.name == "char"` apart from a chord without comparing against a list.
///
/// **The names are the decoded ones.** `term` folds Ctrl-A into [`Key::Home`] before anything here
/// runs, so that key is reported as `"home"` — the same name `oslo.keys` uses for it, which is the
/// property worth keeping. A hook cannot recover the chord, and neither can a binding.
pub(super) fn hook_key_name(key: Key) -> (String, Option<char>) {
    match key {
        Key::Char(c) => ("char".to_string(), Some(c)),
        Key::Ctrl(c) => (format!("ctrl-{c}"), None),
        Key::Alt(c) if !c.is_control() => (format!("alt-{c}"), None),
        Key::Alt(_) => ("alt-backspace".to_string(), None),
        Key::Accept => ("enter".to_string(), None),
        Key::Cancel => ("esc".to_string(), None),
        Key::Abort => ("ctrl-c".to_string(), None),
        Key::Backspace => ("backspace".to_string(), None),
        Key::Delete => ("delete".to_string(), None),
        Key::ToggleScope => ("tab".to_string(), None),
        Key::BackTab => ("shift-tab".to_string(), None),
        Key::Clear => ("ctrl-u".to_string(), None),
        Key::Up => ("up".to_string(), None),
        Key::Down => ("down".to_string(), None),
        Key::Left => ("left".to_string(), None),
        Key::Right => ("right".to_string(), None),
        Key::Home => ("home".to_string(), None),
        Key::End => ("end".to_string(), None),
        Key::PageUp => ("pageup".to_string(), None),
        Key::PageDown => ("pagedown".to_string(), None),
        Key::Function(number @ 1..=12) => (format!("f{number}"), None),
        Key::Function(_) => ("ignored".to_string(), None),
        // Reported, because a hook watching keystrokes may reasonably want to know the window
        // moved under it — but it is not a key and is named so that no config mistakes it for one.
        Key::Resized => ("resized".to_string(), None),
        Key::Ignored => ("ignored".to_string(), None),
    }
}
