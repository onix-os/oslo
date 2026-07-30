//! Evaluating expressions.
//!
//! One thing shapes this whole file: in Lua an expression can produce **any number of values**.
//! `f()` is one value in `local x = f()`, all of them in `g(f())`, and none at all if `f` returns
//! nothing. So there are two entry points — [`eval`] for the single-value positions and
//! [`eval_multi`] for the positions that take everything — and mixing them up is how `print(f())`
//! ends up printing only the first result.

use super::scope::{Closure, Scope};
use super::value::{Table, Value};
use super::{Interp, LuaError, LuaResult, ops};
use full_moon::ast::{
    Call, Expression, Field, FunctionArgs, FunctionBody, Index, Parameter, Prefix, Suffix,
    TableConstructor, Var,
};
use full_moon::tokenizer::{TokenReference, TokenType};
use std::rc::Rc;

/// The line a token sits on, for diagnostics.
pub fn line_of(token: &TokenReference) -> usize {
    token.start_position().line()
}

/// Evaluate an expression to exactly one value.
///
/// A call in this position is truncated to its first result, and to nil when it returned nothing —
/// which is Lua's rule, not a convenience.
pub fn eval(interp: &mut Interp, expr: &Expression, scope: &Rc<Scope>) -> LuaResult<Value> {
    Ok(first(eval_multi(interp, expr, scope)?))
}

/// Evaluate an expression to all the values it produces.
pub fn eval_multi(
    interp: &mut Interp,
    expr: &Expression,
    scope: &Rc<Scope>,
) -> LuaResult<Vec<Value>> {
    match expr {
        Expression::Number(token) => {
            let text = token.token().to_string();
            super::value::parse_number(&text)
                .map(|n| vec![Value::Number(n)])
                .ok_or_else(|| LuaError::new(format!("malformed number near '{text}'")))
        }

        Expression::String(token) => Ok(vec![Value::str(string_literal(token))]),

        Expression::Symbol(token) => {
            let text = token.token().to_string();
            Ok(vec![match text.as_str() {
                "nil" => Value::Nil,
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                // `...` is the only symbol that is plural, and it is the reason `eval_multi`
                // exists at all.
                "..." => return Ok(interp.varargs.clone()),
                other => {
                    return Err(
                        LuaError::new(format!("unexpected symbol '{other}'")).at(line_of(token))
                    );
                }
            }])
        }

        Expression::Parentheses { expression, .. } => {
            // Parentheses are not decoration: `(f())` truncates a multi-value call to one value.
            Ok(vec![eval(interp, expression, scope)?])
        }

        Expression::UnaryOperator { unop, expression } => {
            let operand = eval(interp, expression, scope)?;
            let op = unop.to_string();
            Ok(vec![ops::unary(interp, op.trim(), &operand)?])
        }

        Expression::BinaryOperator { lhs, binop, rhs } => {
            Ok(vec![binary(interp, lhs, &binop.to_string(), rhs, scope)?])
        }

        Expression::TableConstructor(ctor) => Ok(vec![table_constructor(interp, ctor, scope)?]),

        Expression::Function(boxed) => {
            let (_, body) = &**boxed;
            Ok(vec![make_closure(body, scope, None)])
        }

        Expression::FunctionCall(call) => {
            let prefix = eval_prefix(interp, call.prefix(), scope)?;
            apply_suffixes(interp, prefix, call.suffixes(), scope)
        }

        Expression::Var(var) => Ok(vec![eval_var(interp, var, scope)?]),

        other => Err(unsupported(&format!("expression `{other}`"))),
    }
}

/// Binary operators, with `and`/`or` short-circuiting before either side is forced.
fn binary(
    interp: &mut Interp,
    lhs: &Expression,
    op: &str,
    rhs: &Expression,
    scope: &Rc<Scope>,
) -> LuaResult<Value> {
    let op = op.trim();

    // Evaluated first and separately: `a and b` must not evaluate `b` when `a` is false, and
    // `x ~= nil and x.field` is the idiom that depends on it.
    match op {
        "and" => {
            let left = eval(interp, lhs, scope)?;
            return if left.truthy() {
                eval(interp, rhs, scope)
            } else {
                Ok(left)
            };
        }
        "or" => {
            let left = eval(interp, lhs, scope)?;
            return if left.truthy() {
                Ok(left)
            } else {
                eval(interp, rhs, scope)
            };
        }
        _ => {}
    }

    let a = eval(interp, lhs, scope)?;
    let b = eval(interp, rhs, scope)?;
    match op {
        "==" => Ok(Value::Bool(ops::equals(interp, &a, &b)?)),
        "~=" => Ok(Value::Bool(!ops::equals(interp, &a, &b)?)),
        "<" | "<=" => Ok(Value::Bool(ops::compare(interp, op, &a, &b)?)),
        ">" => Ok(Value::Bool(ops::compare(interp, "<", &b, &a)?)),
        ">=" => Ok(Value::Bool(ops::compare(interp, "<=", &b, &a)?)),
        ".." => ops::concat(interp, &a, &b),
        arithmetic => ops::arith(interp, arithmetic, &a, &b),
    }
}

/// A variable reference: a bare name, or a prefix with `.`/`[]` suffixes.
pub fn eval_var(interp: &mut Interp, var: &Var, scope: &Rc<Scope>) -> LuaResult<Value> {
    match var {
        Var::Name(name) => Ok(lookup(interp, &name.token().to_string(), scope)),
        Var::Expression(expression) => {
            let prefix = eval_prefix(interp, expression.prefix(), scope)?;
            Ok(first(apply_suffixes(
                interp,
                prefix,
                expression.suffixes(),
                scope,
            )?))
        }
        other => Err(unsupported(&format!("variable `{other}`"))),
    }
}

/// Read a name: locals first, then globals.
///
/// The fall-through to globals is what lets Lua's own stdlib live in `_G` while a `local print`
/// shadows it — and it is the same shape the shell-variable bridge will hang off later.
pub fn lookup(interp: &Interp, name: &str, scope: &Rc<Scope>) -> Value {
    scope
        .get(name)
        .unwrap_or_else(|| interp.globals.borrow().get(&Value::str(name)))
}

fn eval_prefix(interp: &mut Interp, prefix: &Prefix, scope: &Rc<Scope>) -> LuaResult<Value> {
    match prefix {
        Prefix::Name(name) => Ok(lookup(interp, &name.token().to_string(), scope)),
        Prefix::Expression(expression) => eval(interp, expression, scope),
        other => Err(unsupported(&format!("prefix `{other}`"))),
    }
}

/// Walk `.field`, `[key]`, `(args)` and `:method(args)` left to right.
fn apply_suffixes(
    interp: &mut Interp,
    mut current: Value,
    suffixes: impl Iterator<Item = impl std::borrow::Borrow<Suffix>>,
    scope: &Rc<Scope>,
) -> LuaResult<Vec<Value>> {
    let mut multi: Option<Vec<Value>> = None;

    for suffix in suffixes {
        // Only the *last* suffix may produce several values; anything a further suffix is applied
        // to has already been narrowed to one.
        if let Some(values) = multi.take() {
            current = first(values);
        }
        match suffix.borrow() {
            Suffix::Index(Index::Dot { name, .. }) => {
                let key = Value::str(name.token().to_string());
                current = ops::index(interp, &current, &key).map_err(|e| e.at(line_of(name)))?;
            }
            Suffix::Index(Index::Brackets { expression, .. }) => {
                let key = eval(interp, expression, scope)?;
                current = ops::index(interp, &current, &key)?;
            }
            Suffix::Call(Call::AnonymousCall(args)) => {
                let argv = eval_args(interp, args, scope)?;
                multi = Some(interp.call(&current, argv)?);
            }
            Suffix::Call(Call::MethodCall(method)) => {
                // `obj:m(a)` is `obj.m(obj, a)`. The receiver is evaluated once — writing it out
                // as the sugar would evaluate the prefix twice.
                let key = Value::str(method.name().token().to_string());
                let function = ops::index(interp, &current, &key)?;
                let mut argv = vec![current.clone()];
                argv.extend(eval_args(interp, method.args(), scope)?);
                multi = Some(
                    interp
                        .call(&function, argv)
                        .map_err(|e| e.at(line_of(method.name())))?,
                );
            }
            other => return Err(unsupported(&format!("suffix `{other}`"))),
        }
    }

    Ok(multi.unwrap_or_else(|| vec![current]))
}

/// The argument list of a call, in any of Lua's three spellings.
fn eval_args(interp: &mut Interp, args: &FunctionArgs, scope: &Rc<Scope>) -> LuaResult<Vec<Value>> {
    match args {
        FunctionArgs::Parentheses { arguments, .. } => {
            let mut out = Vec::new();
            let total = arguments.len();
            for (i, expression) in arguments.iter().enumerate() {
                // Only the final argument expands to all its values: `f(g(), h())` passes one
                // value from `g` and everything from `h`.
                if i + 1 == total {
                    out.extend(eval_multi(interp, expression, scope)?);
                } else {
                    out.push(eval(interp, expression, scope)?);
                }
            }
            Ok(out)
        }
        // `f"str"` and `f{...}` — the sugar that makes `require "x"` work.
        FunctionArgs::String(token) => Ok(vec![Value::str(string_literal(token))]),
        FunctionArgs::TableConstructor(ctor) => Ok(vec![table_constructor(interp, ctor, scope)?]),
        other => Err(unsupported(&format!("call arguments `{other}`"))),
    }
}

/// `{ 1, 2, x = 3, [k] = v }`.
fn table_constructor(
    interp: &mut Interp,
    ctor: &TableConstructor,
    scope: &Rc<Scope>,
) -> LuaResult<Value> {
    let mut table = Table::new();
    let mut next_index = 1i64;
    let total = ctor.fields().len();

    for (i, field) in ctor.fields().iter().enumerate() {
        match field {
            Field::NoKey(expression) => {
                // A trailing call or `...` spreads: `{f()}` collects every result.
                if i + 1 == total {
                    for value in eval_multi(interp, expression, scope)? {
                        table.set(Value::int(next_index), value);
                        next_index += 1;
                    }
                } else {
                    table.set(Value::int(next_index), eval(interp, expression, scope)?);
                    next_index += 1;
                }
            }
            Field::NameKey { key, value, .. } => {
                table.set(
                    Value::str(key.token().to_string()),
                    eval(interp, value, scope)?,
                );
            }
            Field::ExpressionKey { key, value, .. } => {
                let k = eval(interp, key, scope)?;
                if matches!(k, Value::Nil) {
                    return Err(LuaError::new("table index is nil"));
                }
                table.set(k, eval(interp, value, scope)?);
            }
            other => return Err(unsupported(&format!("table field `{other}`"))),
        }
    }

    Ok(Value::table(table))
}

/// Build a function value that closes over `scope`.
pub fn make_closure(body: &FunctionBody, scope: &Rc<Scope>, name: Option<Rc<str>>) -> Value {
    let mut params = Vec::new();
    let mut varargs = false;
    for parameter in body.parameters() {
        match parameter {
            Parameter::Name(token) => params.push(Rc::from(token.token().to_string().as_str())),
            Parameter::Ellipsis(_) => varargs = true,
            _ => {}
        }
    }

    Value::Function(Rc::new(super::value::Function::Lua(Closure {
        params,
        varargs,
        body: Rc::new(body.clone()),
        captured: Rc::clone(scope),
        name,
    })))
}

/// The text of a string literal, with quotes removed and escapes resolved.
fn string_literal(token: &TokenReference) -> String {
    match token.token_type() {
        TokenType::StringLiteral {
            literal,
            multi_line_depth,
            quote_type,
        } => {
            // A long string (`[[...]]`) takes no escapes at all — its contents are literal, which
            // is exactly why scripts use it for regexes and Windows paths.
            if *multi_line_depth > 0
                || matches!(
                    quote_type,
                    full_moon::tokenizer::StringLiteralQuoteType::Brackets
                )
            {
                return literal.to_string();
            }
            unescape(literal)
        }
        // Not a string token; the parser guarantees this is unreachable for a string expression,
        // but rendering the token is a better answer than a panic if it ever is.
        _ => token.to_string(),
    }
}

/// Resolve Lua's backslash escapes.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some('v') => out.push('\x0b'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('\n') => out.push('\n'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The first of a value list, or nil.
pub fn first(mut values: Vec<Value>) -> Value {
    if values.is_empty() {
        Value::Nil
    } else {
        values.remove(0)
    }
}

/// The house style for a construct oslo's Lua does not implement.
///
/// Loud and specific, never silent: see the module comment on `super`.
pub fn unsupported(what: &str) -> LuaError {
    LuaError::new(format!("{what} is not implemented in oslo's Lua"))
}
