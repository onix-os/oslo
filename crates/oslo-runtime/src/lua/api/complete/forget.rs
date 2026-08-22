//! `oslo.completion.forget(name)` — the other half of registering one.
//!
//! # Why it did not exist
//!
//! Three surfaces add completions and none of them could take one away by name. Both Rust
//! registries offered an argument-less `forget()` that cleared *everything* — and the provider one
//! had no caller anywhere in the tree — while `for_command` is a plain Lua table a config could
//! already nil a key in. So the only supported way to withdraw one completion was to withdraw all
//! of them.
//!
//! That is the wrong shape for a directory environment. Leaving a project should take away the
//! completions that project added and nothing else, which is what `oslo.direnv.on_unload` is for.
//!
//! # One name, three registries
//!
//! A caller does not know which of the three holds a name, and should not have to: a provider is
//! named for itself (`"tldr"`), a spec and a `for_command` entry are named for the command they
//! complete (`"git"`). One call clears whichever of them answers.
//!
//! The answer is the **count**, not a boolean. `0` says nothing was registered under that name,
//! which is the mistake a caller wants told about.

use super::super::util::{put, text};
use oslo_base::value::{Table, Value};
use oslo_luavm::Host;

/// Add `forget` to the `oslo.completion` table.
pub(in crate::lua::api) fn install(completion: &mut Table) {
    put(completion, "forget", move |host, args| {
        let name = text(&args, 1, "oslo.completion.forget")?;
        let mut gone = 0;
        if oslo_ui::completion::provider::forget_named(&name) {
            gone += 1;
        }
        if oslo_ui::spec::custom::forget_named(&name) {
            gone += 1;
        }
        // `for_command` is a plain Lua table, read live on every Tab, so removing a key is all
        // there is to it.
        //
        // **Through the host, not through a table this side holds.** A table crossing into the VM
        // is a *copy* — the fact `oslo.nix`'s helpers are installed around — so an `Rc` from here
        // is not the table Lua reads, and writing to it would clear nothing while reporting
        // success. Measured: the first attempt left `for_command.gadget` in place and said it had
        // removed it.
        if holds(host, &name) {
            host.set_field(&["oslo", "completion", "for_command", &name], Value::Nil);
            gone += 1;
        }
        Ok(vec![Value::int(gone)])
    });
}

/// Whether `oslo.completion.for_command` holds an entry for `name`.
///
/// **Asked before it is cleared**, because `set_field` answers "the write landed" and not
/// "something was there" — counting it directly made `forget` on an unheard-of name report 1.
///
/// The read copies the table it walks, for the same boundary reason the write goes through the
/// host. That is why this is reached only from `forget`, which happens on unload, and never from
/// anything on the Tab path.
fn holds(host: &dyn Host, name: &str) -> bool {
    let Value::Table(oslo) = host.global("oslo") else {
        return false;
    };
    let completion = oslo.borrow().get_str("completion");
    let Value::Table(completion) = completion else {
        return false;
    };
    let commands = completion.borrow().get_str("for_command");
    match commands {
        Value::Table(commands) => !matches!(commands.borrow().get_str(name), Value::Nil),
        _ => false,
    }
}
