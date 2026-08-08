//! Lexical scopes and closures.
//!
//! Lua closes over *variables*, not values. This is the difference the model has to get right:
//!
//! ```lua
//! local n = 0
//! local function bump() n = n + 1 end
//! bump(); print(n)   --> 1
//! ```
//!
//! `bump` and the enclosing block share one `n`, so a scope holds `RefCell<…>` and a closure holds
//! an `Rc` to the scope chain it was defined in. Copying values into the closure instead would
//! print `0`, and the bug would only show up in code that mutates a captured variable — which is
//! most callback code.
//!
//! Names resolve by walking the chain and then falling back to globals. Real Lua resolves every
//! local to a register index while compiling, which is much faster; this evaluator is glue for a
//! shell, and a hash lookup per variable is not what will make it slow.

use super::value::Value;
use full_moon::ast::FunctionBody;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// One lexical block's locals, linked to the block that encloses it.
#[derive(Debug, Default)]
pub struct Scope {
    vars: RefCell<HashMap<Rc<str>, Value>>,
    parent: Option<Rc<Scope>>,
}

impl Scope {
    /// The outermost scope of a chunk.
    pub fn root() -> Rc<Scope> {
        Rc::new(Scope::default())
    }

    /// A new block inside `parent`.
    pub fn child(parent: &Rc<Scope>) -> Rc<Scope> {
        Rc::new(Scope {
            vars: RefCell::new(HashMap::new()),
            parent: Some(Rc::clone(parent)),
        })
    }

    /// Declare a local *in this scope*, shadowing any outer one.
    ///
    /// Always inserts here rather than assigning through the chain — that is exactly what makes
    /// `local x = x` work, reading the outer `x` to initialise a new inner one.
    pub fn declare(&self, name: Rc<str>, value: Value) {
        self.vars.borrow_mut().insert(name, value);
    }

    /// Read a name, walking outward. `None` means it is not a local anywhere in the chain, so the
    /// caller falls back to globals.
    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.vars.borrow().get(name) {
            return Some(v.clone());
        }
        self.parent.as_ref().and_then(|p| p.get(name))
    }

    /// Assign to an existing local, walking outward. `false` means no local owns this name, so the
    /// caller writes a global instead.
    pub fn set(&self, name: &str, value: Value) -> bool {
        if let Some(slot) = self.vars.borrow_mut().get_mut(name) {
            *slot = value;
            return true;
        }
        match &self.parent {
            Some(p) => p.set(name, value),
            None => false,
        }
    }
}

/// A Lua function value: its source, and the scope chain it captured.
#[derive(Clone)]
pub struct Closure {
    /// Parameter names in order.
    pub params: Vec<Rc<str>>,
    /// Whether the parameter list ended in `...`.
    pub varargs: bool,
    /// The body, shared because a function value can be copied freely.
    pub body: Rc<FunctionBody>,
    /// The scope the function was *defined* in, which is what makes it a closure rather than a
    /// plain function pointer.
    pub captured: Rc<Scope>,
    /// For diagnostics: `function 'name'` in a traceback.
    pub name: Option<Rc<str>>,
}

impl std::fmt::Debug for Closure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "closure {}({} params{})",
            self.name.as_deref().unwrap_or("?"),
            self.params.len(),
            if self.varargs { ", ..." } else { "" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inner_scope_sees_outer_names() {
        let outer = Scope::root();
        outer.declare(Rc::from("x"), Value::int(1));
        let inner = Scope::child(&outer);
        assert!(matches!(inner.get("x"), Some(Value::Number(_))));
        assert!(inner.get("nope").is_none());
    }

    /// The reason scopes hold `RefCell` rather than values being copied: assigning through an
    /// inner scope has to reach the variable the outer one owns.
    #[test]
    fn assignment_reaches_the_scope_that_owns_the_name() {
        let outer = Scope::root();
        outer.declare(Rc::from("n"), Value::int(0));
        let inner = Scope::child(&outer);

        assert!(inner.set("n", Value::int(5)), "an owning scope was found");
        let Some(Value::Number(n)) = outer.get("n") else {
            panic!("outer scope lost the variable");
        };
        assert_eq!(n.as_int(), Some(5), "the write landed in the outer scope");
    }

    /// `local x = 1` inside a block must not overwrite the outer `x`.
    #[test]
    fn declaring_shadows_rather_than_assigns() {
        let outer = Scope::root();
        outer.declare(Rc::from("x"), Value::int(1));
        let inner = Scope::child(&outer);
        inner.declare(Rc::from("x"), Value::int(2));

        let (Some(Value::Number(i)), Some(Value::Number(o))) = (inner.get("x"), outer.get("x"))
        else {
            panic!("both scopes should hold a number");
        };
        assert_eq!((i.as_int(), o.as_int()), (Some(2), Some(1)));
    }

    #[test]
    fn assigning_an_unknown_name_reports_no_owner() {
        let scope = Scope::root();
        assert!(
            !scope.set("undeclared", Value::int(1)),
            "caller falls back to globals"
        );
    }
}
