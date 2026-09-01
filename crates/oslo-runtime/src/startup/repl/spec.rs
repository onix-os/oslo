//! Wiring completion specs to the things only the shell can do.
//!
//! Two hooks, installed once as the shell starts:
//!
//! * **the macro runner**, so a position declaring `$(git branch)` gets an answer. In every build,
//!   because a Lua-declared spec can name one and the reader for `.yaml` files is what the `spec`
//!   feature gates — not the macros.
//! * **the loader**, so `~/.config/oslo/completion/mycmd.yaml` is found the first time `mycmd` is
//!   completed. Only where there is a reader for it.
//! * **the recipe source**, so `make <Tab>` offers what this project declared. Not a file and not
//!   a description of one: the `.make.lua` being completed for *is* the spec — see [`recipes`].

// `.make.lua` is the `make` feature's file, so the source that reads it is that feature's too.
#[cfg(feature = "make")]
#[path = "spec/recipes.rs"]
mod recipes;

/// Register them, once, as the shell starts.
pub(super) fn register() {
    #[cfg(feature = "make")]
    recipes::register();
    oslo_ui::spec::action::set_runner(Some(std::rc::Rc::new(oslo_shell::spec::run::offers)));
    #[cfg(feature = "compgen")]
    oslo_ui::spec::custom::set_loader(Some(std::rc::Rc::new(oslo_shell::spec::find)));
}
