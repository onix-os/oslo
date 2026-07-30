//! `require`, `dofile`, `load` and the `package` table.
//!
//! Nothing about loading a module needed a virtual machine: read the file, parse it with
//! `full_moon`, evaluate it, remember the result. That is the whole implementation, and it is why
//! removing the C interpreter cost no module support.
//!
//! Two deliberate differences from stock Lua, both because of what oslo *is*:
//!
//! * **`package.path` has no `./?.lua`.** Stock Lua searches the working directory, so a `require
//!   "utils"` picks up whatever `utils.lua` happens to be in the directory you ran the command
//!   from. In a shell that is a script hijack: `cd` somewhere untrusted, run a tool, and the tool
//!   loads a stranger's code.
//! * **`package.cpath` is empty.** oslo ships as a static binary and cannot `dlopen` anything.
//!   Advertising a path would turn an honest "module not found" into a confusing loader error.

use super::super::value::{Table, Value};
use super::super::{Interp, LuaError, LuaResult};
use super::{arg_str, module, native, opt_str};
use std::rc::Rc;

/// Where a module is looked for, in order. Notably not the working directory.
const DEFAULT_PATH: &str = "/usr/local/share/lua/5.4/?.lua;/usr/local/share/lua/5.4/?/init.lua;\
     /usr/share/lua/5.4/?.lua;/usr/share/lua/5.4/?/init.lua";

pub fn install(interp: &Interp) {
    let package = module(vec![
        ("loaded", Value::table(Table::new())),
        // `preload` lets a host register a module before anything asks for it, and is the only
        // way to provide one that has no file.
        ("preload", Value::table(Table::new())),
        ("path", Value::str(DEFAULT_PATH)),
        ("cpath", Value::str("")),
    ]);
    interp.set_global("package", package);

    interp.set_global("require", native("require", require));
    interp.set_global("dofile", native("dofile", dofile));
    interp.set_global("loadfile", native("loadfile", loadfile));
    interp.set_global("load", native("load", load));
}

/// A field of the `package` table.
fn package_field(interp: &Interp, name: &str) -> Value {
    let Value::Table(package) = interp.global("package") else {
        return Value::Nil;
    };
    let field = Value::str(name);
    package.borrow().get(&field)
}

fn require(interp: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let name = arg_str(&args, 1, "require")?;

    let Value::Table(loaded) = package_field(interp, "loaded") else {
        return Err(LuaError::new("package.loaded is not a table"));
    };
    // Already loaded: hand back the same value, which is what makes a module a singleton.
    let cached = loaded.borrow().get(&Value::str(&name));
    if !matches!(cached, Value::Nil) {
        // The sentinel below means this module is part-way through loading *and* asked for
        // itself, directly or through something it required.
        if matches!(&cached, Value::Str(s) if &**s == LOADING) {
            return Err(LuaError::new(format!(
                "loop detected while loading module '{name}'"
            )));
        }
        return Ok(vec![cached]);
    }

    // `package.preload` wins over the filesystem, which is how a host-provided module shadows a
    // file of the same name rather than racing it.
    if let Value::Table(preload) = package_field(interp, "preload") {
        let loader = preload.borrow().get(&Value::str(&name));
        if !matches!(loader, Value::Nil) {
            let produced = interp.call(&loader, vec![Value::str(&name)])?;
            let value = produced.into_iter().next().unwrap_or(Value::Bool(true));
            loaded.borrow_mut().set(Value::str(&name), value.clone());
            return Ok(vec![value, Value::str(":preload:")]);
        }
    }

    let path = search(interp, &name)?;
    loaded
        .borrow_mut()
        .set(Value::str(&name), Value::str(LOADING));

    let value = match run_file(interp, &path, vec![Value::str(&name), Value::str(&path)]) {
        Ok(values) => values.into_iter().next().unwrap_or(Value::Bool(true)),
        Err(e) => {
            // The sentinel must not outlive a failed load, or a second `require` after fixing the
            // file would report a loop instead of trying again.
            loaded.borrow_mut().set(Value::str(&name), Value::Nil);
            return Err(e);
        }
    };
    // A module that returns nothing still counts as loaded — `true` is Lua's marker for it, and
    // without it every `require` of such a module would re-run the file.
    let value = if matches!(value, Value::Nil) {
        Value::Bool(true)
    } else {
        value
    };
    loaded.borrow_mut().set(Value::str(&name), value.clone());
    Ok(vec![value, Value::str(path)])
}

/// Marks a module as being loaded right now, so a cycle is reported instead of recursing.
const LOADING: &str = "\0oslo:loading";

/// Turn `a.b` into a path by trying each `?`-pattern in `package.path`.
fn search(interp: &Interp, name: &str) -> LuaResult<String> {
    // A dot is a directory separator in a module name: `require "a.b"` looks for `a/b.lua`.
    let relative = name.replace('.', "/");
    let Value::Str(patterns) = package_field(interp, "path") else {
        return Err(LuaError::new("package.path is not a string"));
    };

    let mut tried = Vec::new();
    for pattern in patterns.split(';') {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            continue;
        }
        let candidate = pattern.replace('?', &relative);
        if std::path::Path::new(&candidate).is_file() {
            return Ok(candidate);
        }
        tried.push(format!("\n\tno file '{candidate}'"));
    }
    // Every place that was looked, because "module not found" without the search path is the
    // least actionable error in Lua.
    Err(LuaError::new(format!(
        "module '{name}' not found:{}",
        tried.join("")
    )))
}

/// Read, parse and evaluate a file as a chunk named after itself.
fn run_file(interp: &Interp, path: &str, varargs: Vec<Value>) -> LuaResult<Vec<Value>> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| LuaError::new(format!("cannot open '{path}': {e}")))?;
    let ast = super::super::parse(&source).map_err(|e| e.in_chunk(path))?;

    // The chunk name and the varargs both belong to the file being run, and both have to come
    // back afterwards — a `require` in the middle of a script must not leave the outer script's
    // errors pointing at the module.
    let outer_chunk = interp.chunk_name();
    let outer_varargs = interp.varargs();
    interp.set_chunk(path);
    interp.set_varargs(varargs);
    let result = interp.run_ast(&ast).map_err(|e| e.in_chunk(path));
    interp.set_chunk(outer_chunk);
    interp.set_varargs(outer_varargs);
    result
}

fn dofile(interp: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let path = arg_str(&args, 1, "dofile")?;
    // No cache and no search path: `dofile` runs the file at the name it was given, every time.
    run_file(interp, &path, Vec::new())
}

fn loadfile(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let path = arg_str(&args, 1, "loadfile")?;
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        // `loadfile` answers rather than raising, which is what makes it the way to try a file
        // that may not be there.
        Err(e) => return Ok(vec![Value::Nil, Value::str(format!("{path}: {e}"))]),
    };
    Ok(compile(&source, &path))
}

fn load(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    // The function form — `load(reader)` calling back for each piece — is not supported: it
    // exists for streaming a chunk that does not fit in memory, which a shell never does.
    let source = arg_str(&args, 1, "load")?;
    let name = opt_str(&args, 2, "load")?.unwrap_or_else(|| "=(load)".to_string());
    Ok(compile(&source, &name))
}

/// Parse `source` into a callable, or answer `nil, message` as `load` does.
fn compile(source: &str, name: &str) -> Vec<Value> {
    let ast = match super::super::parse(source) {
        Ok(ast) => ast,
        Err(e) => return vec![Value::Nil, Value::str(e.in_chunk(name).to_string())],
    };

    // The AST is held by the closure and evaluated on each call, so `local f = load(s)` followed
    // by two calls runs the chunk twice — which is what a compiled chunk does.
    let ast = Rc::new(ast);
    let name = name.to_string();
    vec![native("chunk", move |interp, args| {
        let outer_chunk = interp.chunk_name();
        let outer_varargs = interp.varargs();
        interp.set_chunk(&name);
        interp.set_varargs(args);
        let result = interp.run_ast(&ast);
        interp.set_chunk(outer_chunk);
        interp.set_varargs(outer_varargs);
        result
    })]
}
