//! What a segment last drew, kept so a tick does not redraw everything.
//!
//! Without this, an animated prompt is unaffordable. A prompt is rebuilt as a whole — one pass over
//! the list, every `render` called — which is right when the answer could have changed, and
//! ruinous ten times a second: the segment that shells out to `git` would run ten times a second
//! too, to keep a spinner spinning.
//!
//! So each segment's output is kept under its name, and a rebuild re-runs a segment only when it
//! has something new to say:
//!
//! * the *content* generation moved — [`oslo_ui::prompt::invalidate`] was called, meaning the
//!   directory or a variable or the branch changed, and nothing cached is trustworthy;
//! * or the segment asked to be re-run this often, and that long has passed.
//!
//! A tick moves neither, which is the whole reason the prompt has two counters — see
//! `oslo_ui::prompt::animation`.
//!
//! # Keyed by prompt *and* name
//!
//! `oslo.prompt.left` and `oslo.prompt.right` are separate lists that may each hold a segment
//! called `cwd`, and they are not the same segment: they are given different widths to fit in and
//! may be styled apart. One key would have them overwrite each other's cache every frame, which
//! would be slower than no cache at all.
//!
//! # Thread-local
//!
//! Lua runs on the shell thread and only there, so this never crosses one. It is also why the cache
//! can hold rendered text without a lock around it.

use super::Rendered;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

struct Kept {
    /// The reading of [`oslo_ui::prompt::content_generation`] when this was made.
    content: u64,
    /// When it was made, for a segment that asked to be remade on a clock.
    at: Instant,
    rendered: Rendered,
}

thread_local! {
    static KEPT: RefCell<HashMap<(String, String), Kept>> = RefCell::new(HashMap::new());
}

/// What this segment last drew, if it may still be shown.
///
/// `None` means run it: either nothing is kept, or what is kept is older than the last real change,
/// or the segment asked to be run again by now.
pub fn reuse(key: &str, name: &str, every_ms: Option<u64>) -> Option<Rendered> {
    // **A segment with no name cannot be cached.** Two of them would share one entry and take turns
    // overwriting it, and the second would be drawn where the first should be.
    if name.is_empty() {
        return None;
    }
    let content = oslo_ui::prompt::content_generation();
    KEPT.with(|kept| {
        let kept = kept.borrow();
        let held = kept.get(&(key.to_string(), name.to_string()))?;
        if held.content != content {
            return None;
        }
        if let Some(every) = every_ms
            && held.at.elapsed().as_millis() as u64 >= every
        {
            return None;
        }
        Some(held.rendered.clone())
    })
}

/// Keep what a segment drew, under the prompt it was drawn for.
pub fn keep(key: &str, name: &str, rendered: &Rendered) {
    if name.is_empty() {
        return;
    }
    let entry = Kept {
        content: oslo_ui::prompt::content_generation(),
        at: Instant::now(),
        rendered: rendered.clone(),
    };
    KEPT.with(|kept| {
        kept.borrow_mut()
            .insert((key.to_string(), name.to_string()), entry);
    });
}

/// Forget everything, for a test that needs to start from nothing.
///
/// **Test-only, because nothing else needs it.** A config reload replaces the segments, which looks
/// like a reason to clear — but it also invalidates the prompt, and the content generation moving
/// is already the answer to "nothing kept is trustworthy". A second way to say the same thing would
/// be a second thing to remember to call.
#[cfg(test)]
pub fn forget() {
    KEPT.with(|kept| kept.borrow_mut().clear());
}

#[cfg(test)]
#[path = "cache/tests.rs"]
mod tests;
