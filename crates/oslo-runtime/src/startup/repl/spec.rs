//! Wiring completion specs to the things only the shell can do.
//!
//! Two hooks, installed once as the shell starts:
//!
//! * **the macro runner**, so a position declaring `$(git branch)` gets an answer. In every build,
//!   because a Lua-declared spec can name one and the reader for `.yaml` files is what the `spec`
//!   feature gates — not the macros.
//! * **the loader**, so `~/.config/oslo/specs/mycmd.yaml` is found the first time `mycmd` is
//!   completed. Only where there is a reader for it.

/// Register them, once, as the shell starts.
pub(super) fn register() {
    oslo_ui::spec::action::set_runner(Some(std::rc::Rc::new(oslo_shell::spec::run::offers)));
    #[cfg(feature = "spec")]
    oslo_ui::spec::custom::set_loader(Some(std::rc::Rc::new(oslo_shell::spec::find)));
}
