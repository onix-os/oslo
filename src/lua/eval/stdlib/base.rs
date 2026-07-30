//! The base library: the functions that live directly in `_G`.

use super::super::value::{Number, Value, parse_number};
use super::super::{Interp, LuaError, LuaResult, ops};
use super::{arg, arg_int, arg_str, arg_table, native};
use std::io::Write;
use std::rc::Rc;

pub fn install(interp: &mut Interp) {
    interp.set_global("print", native("print", print));
    interp.set_global("type", native("type", lua_type));
    interp.set_global("tostring", native("tostring", tostring));
    interp.set_global("tonumber", native("tonumber", tonumber));
    interp.set_global("ipairs", native("ipairs", ipairs));
    interp.set_global("pairs", native("pairs", pairs));
    interp.set_global("next", native("next", next));
    interp.set_global("error", native("error", error));
    interp.set_global("assert", native("assert", assert));
    interp.set_global("pcall", native("pcall", pcall));
    interp.set_global("xpcall", native("xpcall", xpcall));
    interp.set_global("select", native("select", select));
    interp.set_global("rawget", native("rawget", rawget));
    interp.set_global("rawset", native("rawset", rawset));
    interp.set_global("rawequal", native("rawequal", rawequal));
    interp.set_global("rawlen", native("rawlen", rawlen));
    interp.set_global("setmetatable", native("setmetatable", setmetatable));
    interp.set_global("getmetatable", native("getmetatable", getmetatable));
}

fn print(interp: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let mut line = String::new();
    for (i, value) in args.iter().enumerate() {
        if i > 0 {
            line.push('\t');
        }
        // Through `tostring`, so that `__tostring` is honoured — that is what makes a table with a
        // metatable print as what it represents rather than as an address.
        line.push_str(&ops::tostring(interp, value)?);
    }
    line.push('\n');
    let mut out = std::io::stdout().lock();
    // A write failure is a real error, not something to swallow: `lua -e 'print(x)' | head -1`
    // must report EPIPE the way the shell's own `echo` does.
    out.write_all(line.as_bytes())
        .map_err(|e| LuaError::new(format!("print: {e}")))?;
    Ok(Vec::new())
}

fn lua_type(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    if args.is_empty() {
        return Err(LuaError::new("bad argument #1 to 'type' (value expected)"));
    }
    Ok(vec![Value::str(args[0].type_name())])
}

fn tostring(interp: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let text = ops::tostring(interp, &arg(&args, 1))?;
    Ok(vec![Value::str(text)])
}

fn tonumber(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let value = arg(&args, 1);
    // With a base, the argument is a string of digits in that base and nothing else — notably
    // `tonumber("10", 2)` is 2, while `tonumber("10")` is 10.
    if args.len() >= 2 && !matches!(args[1], Value::Nil) {
        let base = arg_int(&args, 2, "tonumber")?;
        if !(2..=36).contains(&base) {
            return Err(LuaError::new(
                "bad argument #2 to 'tonumber' (base out of range)",
            ));
        }
        let text = arg_str(&args, 1, "tonumber")?;
        return Ok(vec![
            i64::from_str_radix(text.trim(), base as u32)
                .map(Value::int)
                .unwrap_or(Value::Nil),
        ]);
    }
    Ok(vec![match value {
        Value::Number(n) => Value::Number(n),
        Value::Str(s) => parse_number(&s).map(Value::Number).unwrap_or(Value::Nil),
        _ => Value::Nil,
    }])
}

fn ipairs(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let table = arg(&args, 1);
    if matches!(table, Value::Nil) {
        return Err(LuaError::new(
            "bad argument #1 to 'ipairs' (table expected, got no value)",
        ));
    }
    let step = native("ipairs iterator", |interp, args| {
        let index = arg_int(&args, 2, "ipairs")? + 1;
        // Indexed through `ops::index`, so `ipairs` walks a proxy table with `__index` the way
        // real Lua 5.4 does.
        let value = ops::index(interp, &arg(&args, 1), &Value::int(index))?;
        if matches!(value, Value::Nil) {
            return Ok(vec![Value::Nil]);
        }
        Ok(vec![Value::int(index), value])
    });
    Ok(vec![step, table, Value::int(0)])
}

fn pairs(interp: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let table = arg(&args, 1);
    // `__pairs` lets a table define its own iteration; Lua 5.4 still honours it.
    if let Some(handler) = ops::metamethod(&table, "__pairs") {
        let mut produced = interp.call(&handler, vec![table])?;
        produced.resize(3, Value::Nil);
        return Ok(produced);
    }
    let t = arg_table(&args, 1, "pairs")?;
    // The snapshot is taken here rather than walked live, because this evaluator has no stable
    // ordering to resume from and `RefCell` would be held across the loop body otherwise. The
    // visible difference from real Lua: assigning a *new* key during the loop is not seen, where
    // Lua leaves that undefined anyway.
    let entries: Vec<(Value, Value)> = t.borrow().pairs();
    let cursor = std::cell::Cell::new(0usize);
    let step = native("pairs iterator", move |_, _| {
        let i = cursor.get();
        cursor.set(i + 1);
        Ok(match entries.get(i) {
            Some((k, v)) => vec![k.clone(), v.clone()],
            None => vec![Value::Nil],
        })
    });
    Ok(vec![step, Value::Table(t), Value::Nil])
}

fn next(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let t = arg_table(&args, 1, "next")?;
    let entries = t.borrow().pairs();
    let key = arg(&args, 2);
    let position = if matches!(key, Value::Nil) {
        0
    } else {
        match entries.iter().position(|(k, _)| k.lua_eq(&key)) {
            Some(i) => i + 1,
            None => return Err(LuaError::new("invalid key to 'next'")),
        }
    };
    Ok(match entries.get(position) {
        Some((k, v)) => vec![k.clone(), v.clone()],
        None => vec![Value::Nil],
    })
}

fn error(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    // Real Lua propagates the error *value*, so `error({code = 2})` is catchable as a table. This
    // evaluator carries a message only; a non-string is rendered rather than preserved, which is
    // the one place `pcall` is lossy. Recorded in PLAN-LUA.md.
    Err(LuaError::new(arg(&args, 1).to_display()))
}

fn assert(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    if arg(&args, 1).truthy() {
        return Ok(args);
    }
    Err(LuaError::new(match args.get(1) {
        Some(message) => message.to_display(),
        None => "assertion failed!".to_string(),
    }))
}

fn pcall(interp: &mut Interp, mut args: Vec<Value>) -> LuaResult<Vec<Value>> {
    if args.is_empty() {
        return Err(LuaError::new("bad argument #1 to 'pcall' (value expected)"));
    }
    let callee = args.remove(0);
    match interp.call(&callee, args) {
        Ok(values) => {
            let mut out = vec![Value::Bool(true)];
            out.extend(values);
            Ok(out)
        }
        Err(e) => Ok(vec![Value::Bool(false), Value::str(e.to_string())]),
    }
}

fn xpcall(interp: &mut Interp, mut args: Vec<Value>) -> LuaResult<Vec<Value>> {
    if args.len() < 2 {
        return Err(LuaError::new(
            "bad argument #2 to 'xpcall' (value expected)",
        ));
    }
    let callee = args.remove(0);
    let handler = args.remove(0);
    match interp.call(&callee, args) {
        Ok(values) => {
            let mut out = vec![Value::Bool(true)];
            out.extend(values);
            Ok(out)
        }
        Err(e) => {
            // The handler runs after unwinding rather than at the point of the error, so it cannot
            // see the failing frame's locals. Only `debug.traceback` would notice, and that is a
            // stub here anyway.
            let handled = interp.call(&handler, vec![Value::str(e.to_string())])?;
            let mut out = vec![Value::Bool(false)];
            out.extend(handled);
            Ok(out)
        }
    }
}

fn select(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let rest = &args[1.min(args.len())..];
    if let Value::Str(s) = arg(&args, 1)
        && &*s == "#"
    {
        return Ok(vec![Value::int(rest.len() as i64)]);
    }
    let n = arg_int(&args, 1, "select")?;
    if n < 0 {
        // A negative index counts back from the end: `select(-1, ...)` is the last argument.
        let from = rest.len() as i64 + n;
        if from < 0 {
            return Err(LuaError::new(
                "bad argument #1 to 'select' (index out of range)",
            ));
        }
        return Ok(rest[from as usize..].to_vec());
    }
    if n == 0 {
        return Err(LuaError::new(
            "bad argument #1 to 'select' (index out of range)",
        ));
    }
    Ok(rest.get(n as usize - 1..).unwrap_or_default().to_vec())
}

fn rawget(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let t = arg_table(&args, 1, "rawget")?;
    let value = t.borrow().get(&arg(&args, 2));
    Ok(vec![value])
}

fn rawset(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let t = arg_table(&args, 1, "rawset")?;
    t.borrow_mut().set(arg(&args, 2), arg(&args, 3));
    Ok(vec![Value::Table(t)])
}

fn rawequal(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![Value::Bool(arg(&args, 1).lua_eq(&arg(&args, 2)))])
}

fn rawlen(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![match arg(&args, 1) {
        Value::Table(t) => Value::Number(Number::Int(t.borrow().length())),
        Value::Str(s) => Value::int(s.len() as i64),
        other => {
            return Err(LuaError::new(format!(
                "table or string expected, got {}",
                other.type_name()
            )));
        }
    }])
}

fn setmetatable(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let t = arg_table(&args, 1, "setmetatable")?;
    match arg(&args, 2) {
        Value::Nil => t.borrow_mut().metatable = None,
        Value::Table(meta) => t.borrow_mut().metatable = Some(meta),
        other => {
            return Err(LuaError::new(format!(
                "bad argument #2 to 'setmetatable' (nil or table expected, got {})",
                other.type_name()
            )));
        }
    }
    Ok(vec![Value::Table(t)])
}

fn getmetatable(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let Value::Table(t) = arg(&args, 1) else {
        return Ok(vec![Value::Nil]);
    };
    let meta = t.borrow().metatable.clone();
    Ok(vec![match meta {
        // `__metatable` hides the real one, which is how a library makes its objects tamper-proof.
        Some(m) => {
            let guard = m.borrow().get(&Value::str("__metatable"));
            if matches!(guard, Value::Nil) {
                Value::Table(Rc::clone(&m))
            } else {
                guard
            }
        }
        None => Value::Nil,
    }])
}
