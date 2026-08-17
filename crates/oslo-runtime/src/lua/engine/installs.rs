//! Handing pieces of the line editor over to Lua.
//!
//! Each of these is one line of delegation to [`crate::lua::columns`], where the work and the
//! reasoning live. They are together because they are the same *kind* of thing — a capability the
//! editor has that only the interpreter can supply — and apart from [`super::LuaEngine`]'s own
//! methods because none of them is about running Lua.
//!
//! **When each is installed is the part worth knowing**, and it differs:
//!
//! * the column provider and the per-command completer are read *after* the config has run, for
//!   the same reason the theme is: a config may set a function, change its mind, and set another;
//! * the Lua name completer is installed unconditionally, because completing a Lua name against
//!   the names that exist is what the Lua prompt *is*, not something a config switches on. It used
//!   to be installed alongside the other two, and so ran only for a session that had a config file
//!   at all — every fresh `$HOME` had a Lua prompt that completed nothing.

use super::LuaEngine;

impl LuaEngine {
    /// Install `oslo.prompt.columns`, the dropdown's column function.
    pub fn install_column_provider(&self) {
        crate::lua::columns::install(&self.interp);
    }

    /// Install `oslo.completion.for_command`, the per-command completion hook.
    pub fn install_command_completer(&self) {
        crate::lua::columns::install_command_completer(&self.interp);
    }

    /// Let the Lua prompt complete against the names that actually exist in this session.
    ///
    /// The editor knows what is being typed; only the interpreter knows what a name is. This hands
    /// over the second half — see [`oslo_ui::completion::lua`].
    pub fn install_lua_completer(&self) {
        crate::lua::columns::install_lua_completer(&self.interp);
    }
}
