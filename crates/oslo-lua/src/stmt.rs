//! Executing statements.
//!
//! Each block gets its own [`Scope`], which is what makes `local` block-scoped and what closures
//! capture. Control flow that leaves a block — `break`, `return` — travels back as [`Flow`] rather
//! than as an error, so that a `return` inside a `pcall` is a return and not a failure.

use super::expr::{self, first, line_of, unsupported};
use super::scope::Scope;
use super::value::{Number, Value};
use super::{Flow, Interp, LuaError, LuaResult, ops};
use full_moon::ast::{Block, Expression, LastStmt, Stmt, Var};
use std::rc::Rc;

/// Run every statement in a block, in the scope given.
pub fn exec_block(interp: &Interp, block: &Block, scope: &Rc<Scope>) -> LuaResult<Flow> {
    for statement in block.stmts() {
        match exec_stmt(interp, statement, scope)? {
            Flow::Normal => {}
            leaving => return Ok(leaving),
        }
    }

    match block.last_stmt() {
        Some(LastStmt::Return(ret)) => {
            let mut values = Vec::new();
            let total = ret.returns().len();
            for (i, expression) in ret.returns().iter().enumerate() {
                // `return f()` forwards every value; `return f(), 1` forwards only the first.
                if i + 1 == total {
                    values.extend(expr::eval_multi(interp, expression, scope)?);
                } else {
                    values.push(expr::eval(interp, expression, scope)?);
                }
            }
            Ok(Flow::Return(values))
        }
        Some(LastStmt::Break(_)) => Ok(Flow::Break),
        Some(other) => Err(unsupported(&format!("`{other}`"))),
        None => Ok(Flow::Normal),
    }
}

fn exec_stmt(interp: &Interp, statement: &Stmt, scope: &Rc<Scope>) -> LuaResult<Flow> {
    // Per statement is the granularity Lua itself reports at, and it is what lets a native
    // function — `error`, or any `oslo.*` call — say which line raised it.
    if let Some(position) = full_moon::node::Node::start_position(statement) {
        interp.set_line(position.line());
    }
    match statement {
        Stmt::LocalAssignment(local) => {
            let values = value_list(interp, local.expressions().iter(), scope)?;
            for (i, name) in local.names().iter().enumerate() {
                let value = values.get(i).cloned().unwrap_or(Value::Nil);
                // Declared in *this* scope, so `local x = x` reads the outer one and shadows it.
                scope.declare(Rc::from(name.token().to_string().as_str()), value);
            }
            Ok(Flow::Normal)
        }

        Stmt::Assignment(assignment) => {
            let values = value_list(interp, assignment.expressions().iter(), scope)?;
            for (i, target) in assignment.variables().iter().enumerate() {
                assign(
                    interp,
                    target,
                    values.get(i).cloned().unwrap_or(Value::Nil),
                    scope,
                )?;
            }
            Ok(Flow::Normal)
        }

        Stmt::LocalFunction(local) => {
            let name: Rc<str> = Rc::from(local.name().token().to_string().as_str());
            // Declared *before* the body is built, so the closure captures a scope in which its
            // own name already exists — that is what makes `local function f() return f() end`
            // recurse instead of finding nil.
            scope.declare(Rc::clone(&name), Value::Nil);
            let function = expr::make_closure(local.body(), scope, Some(Rc::clone(&name)));
            scope.declare(name, function);
            Ok(Flow::Normal)
        }

        Stmt::FunctionDeclaration(declaration) => {
            let name = declaration.name();
            let mut parts: Vec<String> =
                name.names().iter().map(|n| n.token().to_string()).collect();
            let method = name.method_name().map(|m| m.token().to_string());
            if let Some(m) = &method {
                parts.push(m.clone());
            }

            let label: Rc<str> = Rc::from(parts.join(".").as_str());
            let mut function = expr::make_closure(declaration.body(), scope, Some(label));

            // `function t.a.b:m()` gains an implicit `self`, and lands in `t.a.b` under `m`.
            if method.is_some() {
                function = with_self(function);
            }

            let head = parts.remove(0);
            if parts.is_empty() {
                if !scope.set(&head, function.clone()) {
                    interp.set_script_global(&head, function);
                }
                return Ok(Flow::Normal);
            }

            let mut target = expr::lookup(interp, &head, scope);
            let last = parts.pop().expect("checked non-empty");
            for part in parts {
                target = ops::index(interp, &target, &Value::str(part))?;
            }
            ops::set_index(interp, &target, Value::str(last), function)?;
            Ok(Flow::Normal)
        }

        Stmt::FunctionCall(call) => {
            // A call as a statement: results are discarded, but the call still happens.
            let expression = Expression::FunctionCall(call.clone());
            expr::eval_multi(interp, &expression, scope)?;
            Ok(Flow::Normal)
        }

        Stmt::Do(block) => {
            let inner = Scope::child(scope);
            exec_block(interp, block.block(), &inner)
        }

        Stmt::If(if_stmt) => {
            if expr::eval(interp, if_stmt.condition(), scope)?.truthy() {
                let inner = Scope::child(scope);
                return exec_block(interp, if_stmt.block(), &inner);
            }
            for clause in if_stmt.else_if().into_iter().flatten() {
                if expr::eval(interp, clause.condition(), scope)?.truthy() {
                    let inner = Scope::child(scope);
                    return exec_block(interp, clause.block(), &inner);
                }
            }
            match if_stmt.else_block() {
                Some(block) => {
                    let inner = Scope::child(scope);
                    exec_block(interp, block, &inner)
                }
                None => Ok(Flow::Normal),
            }
        }

        Stmt::While(while_stmt) => {
            while expr::eval(interp, while_stmt.condition(), scope)?.truthy() {
                let inner = Scope::child(scope);
                match exec_block(interp, while_stmt.block(), &inner)? {
                    Flow::Normal => {}
                    Flow::Break => break,
                    ret => return Ok(ret),
                }
            }
            Ok(Flow::Normal)
        }

        Stmt::Repeat(repeat) => {
            loop {
                // The condition is evaluated *inside* the body's scope: `repeat local x = f()
                // until x` is legal Lua and depends on it.
                let inner = Scope::child(scope);
                match exec_block(interp, repeat.block(), &inner)? {
                    Flow::Normal => {}
                    Flow::Break => break,
                    ret => return Ok(ret),
                }
                if expr::eval(interp, repeat.until(), &inner)?.truthy() {
                    break;
                }
            }
            Ok(Flow::Normal)
        }

        Stmt::NumericFor(numeric) => {
            // **Each bound is evaluated exactly once.** Asking for the number and then asking
            // again whether it was an integer ran `for i = 1, f()` with `f` called twice, and
            // looped to the second answer.
            let start = number(interp, numeric.start(), scope, "'for' initial value")?;
            let limit = number(interp, numeric.end(), scope, "'for' limit")?;
            let step = match numeric.step() {
                Some(e) => number(interp, e, scope, "'for' step")?,
                None => Number::Int(1),
            };
            if step.as_float() == 0.0 {
                return Err(LuaError::new("'for' step is zero"));
            }

            // Kept as f64 for the loop test, but handed to the body as an integer when every part
            // was one — otherwise `for i = 1, 3` would bind `1.0` and `t[i]` would miss.
            let integral = matches!(
                (start, limit, step),
                (Number::Int(_), Number::Int(_), Number::Int(_))
            );
            let (start, limit, step) = (start.as_float(), limit.as_float(), step.as_float());

            let name: Rc<str> = Rc::from(numeric.index_variable().token().to_string().as_str());
            let mut current = start;
            while (step > 0.0 && current <= limit) || (step < 0.0 && current >= limit) {
                let inner = Scope::child(scope);
                let bound = if integral {
                    Value::int(current as i64)
                } else {
                    Value::float(current)
                };
                inner.declare(Rc::clone(&name), bound);
                match exec_block(interp, numeric.block(), &inner)? {
                    Flow::Normal => {}
                    Flow::Break => break,
                    ret => return Ok(ret),
                }
                current += step;
            }
            Ok(Flow::Normal)
        }

        Stmt::GenericFor(generic) => {
            let mut control = value_list(interp, generic.expressions().iter(), scope)?;
            control.resize(3, Value::Nil);
            let (iterator, state) = (control[0].clone(), control[1].clone());
            let mut key = control[2].clone();

            let names: Vec<Rc<str>> = generic
                .names()
                .iter()
                .map(|n| Rc::from(n.token().to_string().as_str()))
                .collect();

            loop {
                let produced = interp.call(&iterator, vec![state.clone(), key.clone()])?;
                let control_value = produced.first().cloned().unwrap_or(Value::Nil);
                // The loop ends when the iterator's *first* result is nil, however many it
                // returned — that is the whole protocol behind `pairs` and `ipairs`.
                if matches!(control_value, Value::Nil) {
                    break;
                }
                key = control_value;

                let inner = Scope::child(scope);
                for (i, name) in names.iter().enumerate() {
                    inner.declare(
                        Rc::clone(name),
                        produced.get(i).cloned().unwrap_or(Value::Nil),
                    );
                }
                match exec_block(interp, generic.block(), &inner)? {
                    Flow::Normal => {}
                    Flow::Break => break,
                    ret => return Ok(ret),
                }
            }
            Ok(Flow::Normal)
        }

        other => Err(unsupported(&format!("statement `{other}`"))),
    }
}

/// Give a method body an implicit `self` as its first parameter.
fn with_self(function: Value) -> Value {
    let Value::Function(f) = &function else {
        return function;
    };
    let super::value::Function::Lua(closure) = &**f else {
        return function;
    };
    let mut updated = closure.clone();
    updated.params.insert(0, Rc::from("self"));
    Value::Function(Rc::new(super::value::Function::Lua(updated)))
}

/// Evaluate a comma-separated expression list, expanding only the last one.
fn value_list<'a>(
    interp: &Interp,
    expressions: impl Iterator<Item = &'a Expression>,
    scope: &Rc<Scope>,
) -> LuaResult<Vec<Value>> {
    let all: Vec<&Expression> = expressions.collect();
    let mut values = Vec::new();
    for (i, expression) in all.iter().enumerate() {
        if i + 1 == all.len() {
            values.extend(expr::eval_multi(interp, expression, scope)?);
        } else {
            values.push(expr::eval(interp, expression, scope)?);
        }
    }
    Ok(values)
}

/// Write to an assignment target: a bare name, or a field of something.
fn assign(interp: &Interp, target: &Var, value: Value, scope: &Rc<Scope>) -> LuaResult<()> {
    match target {
        Var::Name(name) => {
            let text = name.token().to_string();
            // A local anywhere up the chain wins; otherwise this is a global.
            if !scope.set(&text, value.clone()) {
                interp.set_script_global(&text, value);
            }
            Ok(())
        }
        Var::Expression(expression) => {
            // Everything but the final suffix locates the container; the last one is the slot.
            let suffixes: Vec<_> = expression.suffixes().collect();
            let Some((last, leading)) = suffixes.split_last() else {
                return Err(LuaError::new("malformed assignment target"));
            };

            let mut container = match expression.prefix() {
                full_moon::ast::Prefix::Name(name) => {
                    expr::lookup(interp, &name.token().to_string(), scope)
                }
                full_moon::ast::Prefix::Expression(e) => expr::eval(interp, e, scope)?,
                other => return Err(unsupported(&format!("prefix `{other}`"))),
            };
            for suffix in leading {
                container = step(interp, container, suffix, scope)?;
            }

            let key = match last {
                full_moon::ast::Suffix::Index(full_moon::ast::Index::Dot { name, .. }) => {
                    Value::str(name.token().to_string())
                }
                full_moon::ast::Suffix::Index(full_moon::ast::Index::Brackets {
                    expression,
                    ..
                }) => expr::eval(interp, expression, scope)?,
                other => return Err(unsupported(&format!("assignment to `{other}`"))),
            };
            ops::set_index(interp, &container, key, value)
        }
        other => Err(unsupported(&format!("assignment to `{other}`"))),
    }
}

/// Follow one suffix while locating an assignment target.
fn step(
    interp: &Interp,
    current: Value,
    suffix: &full_moon::ast::Suffix,
    scope: &Rc<Scope>,
) -> LuaResult<Value> {
    match suffix {
        full_moon::ast::Suffix::Index(full_moon::ast::Index::Dot { name, .. }) => {
            ops::index(interp, &current, &Value::str(name.token().to_string()))
                .map_err(|e| e.at(line_of(name)))
        }
        full_moon::ast::Suffix::Index(full_moon::ast::Index::Brackets { expression, .. }) => {
            let key = expr::eval(interp, expression, scope)?;
            ops::index(interp, &current, &key)
        }
        other => Err(unsupported(&format!("`{other}` in an assignment target"))),
    }
}

/// A numeric-for bound, as a float, with Lua's error wording.
fn number(
    interp: &Interp,
    expression: &Expression,
    scope: &Rc<Scope>,
    what: &str,
) -> LuaResult<Number> {
    let value = expr::eval(interp, expression, scope)?;
    value
        .as_number()
        .ok_or_else(|| LuaError::new(format!("{what} must be a number")))
}

/// Discard everything but the first value — used where a statement wants one.
#[allow(dead_code)]
fn only(values: Vec<Value>) -> Value {
    first(values)
}
