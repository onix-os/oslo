//! Tools a config adds, registered from above and consulted from here.
//!
//! `oslo.register_tool("k8s", { rows = function(argv) … end })` lets a config supply a data source
//! the shell has never heard of, and `k8s | where phase == "Running"` then works like any built-in
//! one.
//!
//! # Why the table is here rather than beside `register_tool`
//!
//! It used to live in `lua::api::tool`, and `data::tools::run` reached up into the Lua API
//! to ask whether a name was registered. That is the wrong direction: the Lua API is how a config
//! reaches the shell, so it sits *above* it, and the pipeline asking it a question put the shell
//! above the API as well.
//!
//! Inverted the same way the hook registry is. The table is here, where it is read; a handler is an
//! opaque closure, so this knows nothing about what is inside one. `lua::api::tool` puts a closure
//! in that calls Lua, and could as easily be something that does not.
//!
//! Thread-local because a handler is Lua and only the shell's own thread ever runs one.

use super::Record;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// What a registered tool does: turn its arguments into rows, or say why it could not.
pub type Handler = Rc<dyn Fn(&[String]) -> Result<Vec<Record>, String>>;

thread_local! {
    static TOOLS: RefCell<HashMap<String, Handler>> = RefCell::new(HashMap::new());
}

/// Add a tool under `name`, replacing any earlier one of the same name.
pub fn register(name: &str, handler: Handler) {
    TOOLS.with(|slot| slot.borrow_mut().insert(name.to_string(), handler));
}

/// Run the tool called `name`, or `None` if nothing was registered under it.
pub fn rows_of(name: &str, argv: &[String]) -> Option<Result<Vec<Record>, String>> {
    let handler = TOOLS.with(|slot| slot.borrow().get(name).cloned())?;
    Some(handler(argv))
}

/// Whether a tool of this name was registered, without running it.
pub fn registered(name: &str) -> bool {
    TOOLS.with(|slot| slot.borrow().contains_key(name))
}
