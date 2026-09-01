//! How the line editor behaves: the vi keymap, and whether a bracket closes itself.
//!
//! Two settings that answer the same question — what happens to the line as you type it — and that
//! answer it in opposite directions. Vi mode is a *different* way of editing and is off until asked
//! for; autopair is the same way of editing with one keystroke saved, and is on until refused.

/// `oslo.vi` — vi mode, on fish's model.
///
/// ```lua
/// oslo.vi = {
///   enabled = true,
///   cursor_insert = "line",     -- fish's names, so a config need not be translated
///   cursor_normal = "block",
///   cursor_replace = "underscore",
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vi {
    /// **Off by default** — which is what `false` means here, and what every other shell does.
    ///
    /// It was on. A vi user says `oslo.vi.enabled = true` once and never thinks about it again,
    /// whereas the other default made everybody else discover a setting before Esc stopped doing
    /// something surprising, and there are far more of them. There is no flag either way: the
    /// editing mode lives in the config and nowhere else, so a command line and a config file
    /// cannot disagree about it.
    pub enabled: bool,
    pub cursors: crate::vi::Cursors,
}

/// `oslo.autopair`.
///
/// ```lua
/// oslo.autopair.enabled = false   -- type your own closing brackets
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Autopair {
    /// **On by default**, unlike vi mode, and for the opposite reason: this is not a different way
    /// of editing, it is the same way with one keystroke saved. Somebody who does not want it
    /// notices within a line and turns it off; somebody who does want it would never think to look
    /// for a setting that turns on something they assumed was broken.
    pub enabled: bool,
}

impl Default for Autopair {
    fn default() -> Self {
        Self { enabled: true }
    }
}
