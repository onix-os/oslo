//! The `table` library.

use super::super::value::{Table, Value};
use super::super::{Interp, LuaError, LuaResult, ops};
use super::{arg, arg_int, arg_str, arg_table, module, native};

pub fn install(interp: &mut Interp) {
    let unpack_fn = native("table.unpack", unpack);
    let library = module(vec![
        ("insert", native("table.insert", insert)),
        ("remove", native("table.remove", remove)),
        ("concat", native("table.concat", concat)),
        ("sort", native("table.sort", sort)),
        ("unpack", unpack_fn.clone()),
        ("pack", native("table.pack", pack)),
    ]);
    interp.set_global("table", library);
    // `unpack` was a global in 5.1 and moved into `table` in 5.2. Both names point at the one
    // function: the cost is an alias, and the alternative is that every pre-5.2 script fails on
    // its first use.
    interp.set_global("unpack", unpack_fn);
}

fn insert(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let t = arg_table(&args, 1, "insert")?;
    let length = t.borrow().length();
    match args.len() {
        // Two arguments: append.
        2 => t.borrow_mut().set(Value::int(length + 1), arg(&args, 2)),
        3 => {
            let position = arg_int(&args, 2, "insert")?;
            if position < 1 || position > length + 1 {
                return Err(LuaError::new(
                    "bad argument #2 to 'insert' (position out of bounds)",
                ));
            }
            // Shift downwards from the end so no element is overwritten before it is moved.
            let mut table = t.borrow_mut();
            let mut i = length;
            while i >= position {
                let moved = table.get(&Value::int(i));
                table.set(Value::int(i + 1), moved);
                i -= 1;
            }
            table.set(Value::int(position), arg(&args, 3));
        }
        _ => return Err(LuaError::new("wrong number of arguments to 'insert'")),
    }
    Ok(Vec::new())
}

fn remove(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let t = arg_table(&args, 1, "remove")?;
    let length = t.borrow().length();
    let position = match args.get(1) {
        Some(Value::Nil) | None => length,
        _ => arg_int(&args, 2, "remove")?,
    };
    if length == 0 && (position == 0 || position == length) {
        return Ok(vec![Value::Nil]);
    }
    if position < 1 || position > length + 1 {
        return Err(LuaError::new(
            "bad argument #2 to 'remove' (position out of bounds)",
        ));
    }

    let mut table = t.borrow_mut();
    let removed = table.get(&Value::int(position));
    let mut i = position;
    while i < length {
        let moved = table.get(&Value::int(i + 1));
        table.set(Value::int(i), moved);
        i += 1;
    }
    table.set(Value::int(length), Value::Nil);
    Ok(vec![removed])
}

fn concat(interp: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let t = arg_table(&args, 1, "concat")?;
    let separator = match args.get(1) {
        Some(Value::Nil) | None => String::new(),
        _ => arg_str(&args, 2, "concat")?,
    };
    let length = t.borrow().length();
    let from = match args.get(2) {
        Some(Value::Nil) | None => 1,
        _ => arg_int(&args, 3, "concat")?,
    };
    let to = match args.get(3) {
        Some(Value::Nil) | None => length,
        _ => arg_int(&args, 4, "concat")?,
    };

    let mut out = String::new();
    for i in from..=to {
        if i > from {
            out.push_str(&separator);
        }
        let element = t.borrow().get(&Value::int(i));
        match element {
            Value::Str(s) => out.push_str(&s),
            Value::Number(n) => out.push_str(&n.to_string()),
            // Not `tostring`: real Lua refuses here rather than writing `table: 0x…` into the
            // result, and a script that hits this has a bug worth seeing.
            other => {
                return Err(LuaError::new(format!(
                    "invalid value (at index {i}) in table for 'concat' (a {})",
                    other.type_name()
                )));
            }
        }
    }
    let _ = interp;
    Ok(vec![Value::str(out)])
}

fn sort(interp: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let t = arg_table(&args, 1, "sort")?;
    let comparator = arg(&args, 2);
    let length = t.borrow().length();
    let mut items: Vec<Value> = (1..=length)
        .map(|i| t.borrow().get(&Value::int(i)))
        .collect();

    // A merge sort, written out rather than delegated to `slice::sort_by`, because the comparison
    // can call Lua and therefore can fail — and because `sort_by` may panic on an inconsistent
    // comparator, which a script is perfectly able to supply.
    let sorted = merge_sort(interp, &mut items, &comparator)?;
    let mut table = t.borrow_mut();
    for (i, value) in sorted.into_iter().enumerate() {
        table.set(Value::int(i as i64 + 1), value);
    }
    Ok(Vec::new())
}

fn merge_sort(
    interp: &mut Interp,
    items: &mut Vec<Value>,
    comparator: &Value,
) -> LuaResult<Vec<Value>> {
    if items.len() <= 1 {
        return Ok(std::mem::take(items));
    }
    let mut right = items.split_off(items.len() / 2);
    let left = merge_sort(interp, items, comparator)?;
    let right = merge_sort(interp, &mut right, comparator)?;

    let mut out = Vec::with_capacity(left.len() + right.len());
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        if less(interp, comparator, &right[j], &left[i])? {
            out.push(right[j].clone());
            j += 1;
        } else {
            out.push(left[i].clone());
            i += 1;
        }
    }
    out.extend_from_slice(&left[i..]);
    out.extend_from_slice(&right[j..]);
    Ok(out)
}

/// `a < b`, through the script's comparator when it gave one.
fn less(interp: &mut Interp, comparator: &Value, a: &Value, b: &Value) -> LuaResult<bool> {
    if matches!(comparator, Value::Nil) {
        return ops::compare(interp, "<", a, b);
    }
    Ok(interp
        .call(comparator, vec![a.clone(), b.clone()])?
        .first()
        .is_some_and(Value::truthy))
}

fn unpack(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let t = arg_table(&args, 1, "unpack")?;
    let from = match args.get(1) {
        Some(Value::Nil) | None => 1,
        _ => arg_int(&args, 2, "unpack")?,
    };
    let to = match args.get(2) {
        Some(Value::Nil) | None => t.borrow().length(),
        _ => arg_int(&args, 3, "unpack")?,
    };
    if to - from >= 1_000_000 {
        return Err(LuaError::new("too many results to unpack"));
    }
    Ok((from..=to)
        .map(|i| t.borrow().get(&Value::int(i)))
        .collect())
}

fn pack(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let mut table = Table::new();
    let n = args.len();
    for (i, value) in args.into_iter().enumerate() {
        table.set(Value::int(i as i64 + 1), value);
    }
    // `n` is the whole point of `pack`: it records how many arguments there were, including the
    // trailing nils that the sequence length cannot see.
    table.set(Value::str("n"), Value::int(n as i64));
    Ok(vec![Value::table(table)])
}
