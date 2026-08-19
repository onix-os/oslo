//! `oslo.lua` — what the Lua prompt does that the shell prompt does not.

/// What Enter does at a **Lua** prompt. `oslo.lua.enter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Enter {
    /// Run the block as soon as it parses. The default, and what works on every terminal.
    #[default]
    Runs,
    /// Always start another line; an empty line ends the block.
    Newline,
}

/// `oslo.lua` — the Lua prompt's own behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lua {
    /// What Enter does.
    ///
    /// **A setting, never inferred.** oslo briefly chose this from the keyboard-protocol probe:
    /// where Ctrl+Enter could arrive it made Enter add a line. That was still oslo choosing, and it
    /// chose wrong whenever the chord was *grabbed* — by the terminal, by the window manager —
    /// which no probe can see, because a grabbed key is indistinguishable from a supported one.
    /// Every Enter then appended and nothing ever ran.
    ///
    /// So the default is what works everywhere, and a config that knows its terminal says so, with
    /// `oslo.term` to ask what was actually reported:
    ///
    /// ```lua
    /// if oslo.term.kitty_keyboard() then
    ///   oslo.lua.enter = "newline"
    /// end
    /// ```
    ///
    /// An empty line ends a block either way, so neither setting can leave a prompt with no way
    /// out.
    pub enter: Enter,
}
