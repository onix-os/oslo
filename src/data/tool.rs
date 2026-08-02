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
use std::sync::{Mutex, OnceLock};

/// What a registered tool can take and give.
#[derive(Debug, Clone, Copy)]
pub struct Tool {
    pub accepts: Shape,
    pub produces: Shape,
}

fn registry() -> &'static Mutex<HashMap<String, Tool>> {
    static TOOLS: OnceLock<Mutex<HashMap<String, Tool>>> = OnceLock::new();
    TOOLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Declare a tool.
pub fn register(name: &str, accepts: Shape, produces: Shape) {
    if let Ok(mut t) = registry().lock() {
        t.insert(name.to_string(), Tool { accepts, produces });
    }
}

/// What a name declares, or `None` if it declares nothing — which is the answer for every external
/// command and every builtin oslo has today.
pub fn lookup(name: &str) -> Option<Tool> {
    registry().lock().ok()?.get(name).copied()
}

/// Forget every declaration, for a test that wants a known registry.
pub fn clear() {
    if let Ok(mut t) = registry().lock() {
        t.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unregistered name declares nothing, which is what keeps every existing script on the
    /// byte path.
    #[test]
    fn a_name_nobody_registered_declares_nothing() {
        clear();
        assert!(lookup("grep").is_none());
        assert!(lookup("ls").is_none());
        register("where", Shape::Rows, Shape::Rows);
        assert!(lookup("where").is_some());
        assert!(lookup("grep").is_none(), "still nothing");
        clear();
    }
}
