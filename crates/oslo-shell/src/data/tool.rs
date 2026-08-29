//! Which commands understand structure.
//!
//! A tool declares what it takes and what it gives, and those declarations are what the planner
//! reads — never the bytes a command produced, and never a guess from its name.
//!
//! The registry is deliberately small and deliberately *closed to existing names*: every entry is
//! either a name oslo invented or a builtin explicitly declared as bytes. That is what makes the
//! POSIX guarantee mechanical rather than careful — see the `plan` module.

use super::plan::Shape;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// What a registered tool can take and give.
#[derive(Debug, Clone, Copy)]
pub struct Tool {
    pub accepts: Shape,
    pub produces: Shape,
}

/// The columns a config's tool said it produces, or `None` if it did not say.
///
/// The three built-in producers declare theirs in Rust, beside the code that fills them. A tool a
/// config registered had **no way to say at all**, so every one of them was
/// [`Columns::Unknown`](super::columns::Columns::Unknown): no plan-time refusal, no completion. That
/// is exactly backwards — a config's tool is the one that might *do* something on its way to
/// producing rows, so it is the one where catching a typo before it runs is worth most.
fn declared() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static COLUMNS: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    COLUMNS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record the columns a tool answers with. `None` means it did not say.
pub fn declare_columns(name: &str, columns: Option<Vec<String>>) {
    if let Ok(mut slot) = declared().lock() {
        match columns {
            Some(columns) => slot.insert(name.to_string(), columns),
            None => slot.remove(name),
        };
    }
}

/// What a name said it produces, if it said anything.
pub fn columns_of(name: &str) -> Option<Vec<String>> {
    declared().lock().ok()?.get(name).cloned()
}

fn registry() -> &'static Mutex<HashMap<String, Tool>> {
    static TOOLS: OnceLock<Mutex<HashMap<String, Tool>>> = OnceLock::new();
    TOOLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Whether anything has been declared at all.
///
/// **The question asked before the planner does any work.** Planning a pipeline costs a `Vec` per
/// stage, a `String` clone of every command word, a lock on the registry, and an `ioctl` to ask
/// whether stdout is a terminal — and until something is registered the answer is `None` for every
/// pipeline ever written, so all of it was spent to learn nothing. One relaxed load answers it
/// instead, which is the same shape as the feature bitset and the hook registry.
pub fn any_registered() -> bool {
    REGISTERED.load(Ordering::Relaxed)
}

static REGISTERED: AtomicBool = AtomicBool::new(false);

/// Declare a tool.
///
/// The name is also recorded in [`oslo_base::vocab`], which is what the prompt reads: `$PATH` has
/// never heard of `where`, so without it every structured verb was painted as a command that does
/// not exist and offered by nothing on Tab.
pub fn register(name: &str, accepts: Shape, produces: Shape) {
    if let Ok(mut t) = registry().lock() {
        t.insert(name.to_string(), Tool { accepts, produces });
        REGISTERED.store(true, Ordering::Relaxed);
    }
    oslo_base::vocab::add(name, "verb");
}

/// What a name declares, or `None` if it declares nothing — which is the answer for every external
/// command and every builtin oslo has today.
pub fn lookup(name: &str) -> Option<Tool> {
    registry().lock().ok()?.get(name).copied()
}

/// A turn at the registry, for a test that fills or empties it.
///
/// **The registry is process-wide**, so a test that [`clear`]s it cannot run beside one that asks
/// what a name declares. `data::complete`'s tests call `tools::register_all` and then ask whether
/// `parse` is a tool; the test below empties the registry between those two lines about one run in
/// eight, and `parse` stops being a tool for exactly as long as that takes.
///
/// The lock is here, beside the registry, for the same reason
/// [`oslo_base::dirs::named_dirs_turn`] is beside the `@name` table: one piece of shared state, one
/// lock, no chance of two of them each guarding a different nothing.
pub fn registry_turn() -> std::sync::MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Forget every declaration, for a test that wants a known registry.
pub fn clear() {
    if let Ok(mut t) = registry().lock() {
        t.clear();
        REGISTERED.store(false, Ordering::Relaxed);
    }
    oslo_base::vocab::clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unregistered name declares nothing, which is what keeps every existing script on the
    /// byte path.
    #[test]
    fn a_name_nobody_registered_declares_nothing() {
        let _turn = registry_turn();
        clear();
        assert!(lookup("grep").is_none());
        assert!(lookup("ls").is_none());
        register("where", Shape::Rows, Shape::Rows);
        assert!(lookup("where").is_some());
        assert!(lookup("grep").is_none(), "still nothing");
        clear();
    }
}
