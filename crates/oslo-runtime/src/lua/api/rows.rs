//! `oslo.rows` — the structured verbs, as functions.
//!
//! ```lua
//! local rows = oslo.rows.from_json(oslo.run{"docker","ps","--format","json", capture=true}.out)
//! local big  = oslo.rows.where(rows, "size > 1e9")
//! print(oslo.rows.render(oslo.rows.sort_by(big, "name"), "table"))
//! ```
//!
//! # Why these are worth binding
//!
//! `crates/oslo-shell/src/data/` is in every build and is not behind a feature, but its verbs exist
//! only as **pipeline stages** — `ps | where 'rss > 1e9' | sort-by rss`. There is no pipeline in
//! `oslo make` and none inside a registered builtin, so a recipe that wanted rows sorted by a column
//! wrote the sort again in Lua, and got a different answer: `table.sort` compares `"10"` below
//! `"9"`, and the shell's `sort_by` does not.
//!
//! Pure computation over records — no `Environment`, no `borrow_env` — so all of it works from a
//! builtin, a `.env.lua` and a completion provider.
//!
//! # `where` re-enters the VM, and that is the interesting part
//!
//! The filter expression is **Lua**, evaluated per row by
//! `oslo_shell::data::tools::where_::filter`, which asks `oslo_luavm::current::handle()` for the
//! engine. Called from Lua, that is the engine already running — so this is a Lua call inside a Rust
//! call inside Lua. `oslo-luavm` handles it: when the arena is already borrowed it falls back to a
//! re-entrant path rather than panicking. `tests/rows_verb_tests.rs` pins that, because the failure
//! mode if it ever regresses is a panic in somebody's prompt rather than an error they can read.
//!
//! The expression is the shell's, not a Lua callback: `oslo.rows.where(rows, "size > 1024")` is the
//! same string `where` takes at a prompt, so one syntax covers both.

use super::util::{failed, int, ok, opt_text, put, text};
use oslo_base::value::{LuaError, Table, Value};
use oslo_shell::data::Record;
use oslo_shell::data::Val;
use oslo_shell::data::lua::from_lua;
use oslo_shell::data::lua::{records_of, rows_value};
use oslo_shell::data::tools::{bridge, reshape, second, summarise, verbs, where_};

/// Build `oslo.rows`.
pub fn build() -> Value {
    let mut rows = Table::new();
    shaping(&mut rows);
    reshaping(&mut rows);
    positional(&mut rows);
    joining(&mut rows);
    describing(&mut rows);
    grouping(&mut rows);
    reading(&mut rows);
    Value::table(rows)
}

/// Argument one as records, refusing anything that is not a list of tables.
fn input(args: &[Value], owner: &str) -> Result<Vec<Record>, LuaError> {
    match args.first() {
        Some(value @ Value::Table(_)) => Ok(records_of(value)),
        other => Err(LuaError::new(format!(
            "{owner}: argument #1 must be a list of rows, got {}",
            other.map_or("no value", Value::type_name)
        ))),
    }
}

/// A count argument, floored at zero rather than wrapping.
fn count(args: &[Value], n: usize, owner: &str) -> Result<usize, LuaError> {
    Ok(int(args, n, owner)?.max(0) as usize)
}

fn shaping(rows: &mut Table) {
    // oslo.rows.where(rows, expression) -> the rows it is true for, or nil + why
    //
    // **A row whose expression fails is dropped, and the failure is reported once.** Keeping them
    // would mean a broken filter passes everything through, which is how a pipeline ending in `rm`
    // removes the wrong thing. The message is the second return, so a caller may check it.
    put(rows, "where", |_, args| {
        let subject = input(&args, "oslo.rows.where")?;
        let expression = text(&args, 2, "oslo.rows.where")?;
        let (kept, problem) = where_::filter(&subject, &expression);
        match problem {
            Some(why) => Ok(vec![rows_value(&kept), Value::str(why)]),
            None => ok(rows_value(&kept)),
        }
    });

    // oslo.rows.map(rows, expression) -> a row per row, or nil + why
    //
    // The transform the verb list did not have. A row the expression answers `nil` for produces no
    // row, so this filters as well as maps; a failure is the second return, as `where` does it.
    put(rows, "map", |_, args| {
        let subject = input(&args, "oslo.rows.map")?;
        let expression = text(&args, 2, "oslo.rows.map")?;
        let (mapped, problem) = where_::map_rows(&subject, &expression);
        match problem {
            Some(why) => Ok(vec![rows_value(&mapped), Value::str(why)]),
            None => ok(rows_value(&mapped)),
        }
    });

    // oslo.rows.sort_by(rows, column) -> sorted
    //
    // The shell's ordering, which is why this is not `table.sort`: a numeric column sorts as numbers
    // and a size sorts by bytes, where a Lua comparison on the rendered text puts "10" below "9".
    put(rows, "sort_by", |_, args| {
        let subject = input(&args, "oslo.rows.sort_by")?;
        let name = text(&args, 2, "oslo.rows.sort_by")?;
        ok(rows_value(&verbs::sort_by(
            &subject,
            &[name],
            verbs::SortOptions::default(),
        )))
    });

    // oslo.rows.cols(rows, {"a","b"}) -> only those columns, in that order
    put(rows, "cols", |_, args| {
        let subject = input(&args, "oslo.rows.cols")?;
        let names = names_of(args.get(1), "oslo.rows.cols")?;
        ok(rows_value(&verbs::cols(&subject, &names)))
    });

    // oslo.rows.get(rows, column) -> one column, as rows
    put(rows, "get", |_, args| {
        let subject = input(&args, "oslo.rows.get")?;
        let name = text(&args, 2, "oslo.rows.get")?;
        ok(rows_value(&verbs::get(&subject, &name)))
    });

    // oslo.rows.first(rows, n) / oslo.rows.last(rows, n)
    put(rows, "first", |_, args| {
        let subject = input(&args, "oslo.rows.first")?;
        let n = count(&args, 2, "oslo.rows.first")?;
        ok(rows_value(&verbs::first(&subject, n)))
    });
    put(rows, "last", |_, args| {
        let subject = input(&args, "oslo.rows.last")?;
        let n = count(&args, 2, "oslo.rows.last")?;
        ok(rows_value(&verbs::final_rows(&subject, n)))
    });
    // **`final` as well, because that is what the verb is called.** The Rust function is
    // `final_rows` only because `final` is a reserved word there; Lua has no such constraint, and a
    // caller who knows `ls | final 3` should not have to discover that the function is spelled
    // differently. `last` stays: it was the name first, and reads better in Lua.
    put(rows, "final", |_, args| {
        let subject = input(&args, "oslo.rows.final")?;
        let n = count(&args, 2, "oslo.rows.final")?;
        ok(rows_value(&verbs::final_rows(&subject, n)))
    });

    // oslo.rows.length(rows) -> a number
    //
    // **A number, not the one-row table the verb answers with.** `length` keeps the pipeline shape
    // because a pipeline stage has to; a Lua caller wanting `#rows` should not have to write
    // `oslo.rows.length(r)[1].length`.
    put(rows, "length", |_, args| {
        let subject = input(&args, "oslo.rows.length")?;
        ok(Value::int(subject.len() as i64))
    });
}

fn grouping(rows: &mut Table) {
    // oslo.rows.group_by(rows, column) -> one row per distinct value, with its members
    put(rows, "group_by", |_, args| {
        let subject = input(&args, "oslo.rows.group_by")?;
        let field = text(&args, 2, "oslo.rows.group_by")?;
        ok(rows_value(&summarise::group_by(&subject, &field)))
    });

    // oslo.rows.count(rows) -> counts by the columns there are
    put(rows, "count", |_, args| {
        let subject = input(&args, "oslo.rows.count")?;
        ok(rows_value(&summarise::count(&subject)))
    });

    // oslo.rows.distinct(rows, [column])
    put(rows, "distinct", |_, args| {
        let subject = input(&args, "oslo.rows.distinct")?;
        let field = opt_text(&args, 2, "oslo.rows.distinct")?;
        ok(rows_value(&summarise::distinct(&subject, field.as_deref())))
    });

    // oslo.rows.stats(rows, column) -> min, max, sum, mean and the rest, as one row
    put(rows, "stats", |_, args| {
        let subject = input(&args, "oslo.rows.stats")?;
        let field = text(&args, 2, "oslo.rows.stats")?;
        ok(rows_value(&summarise::stats(&subject, &field)))
    });
}

fn reading(rows: &mut Table) {
    // oslo.rows.render(rows, "table"|"text"|"json") -> a string
    //
    // `table` is the rendering a person sees, `text` the one a pipe would have carried, `json` the
    // one another program reads. Defaulting to `table` because a script calling this is almost
    // always about to print it.
    put(rows, "render", |_, args| {
        let subject = input(&args, "oslo.rows.render")?;
        let format = opt_text(&args, 2, "oslo.rows.render")?.unwrap_or_else(|| "table".to_string());
        match verbs::to_format(&subject, &format) {
            Ok(text) => ok(Value::str(text)),
            Err(why) => failed("oslo.rows.render", why),
        }
    });

    // oslo.rows.lines(text) -> one row per line
    put(rows, "lines", |_, args| {
        let input = text(&args, 1, "oslo.rows.lines")?;
        ok(rows_value(&bridge::lines(&input)))
    });

    // oslo.rows.parse(text, pattern) -> rows named by the pattern's captures
    put(rows, "parse", |_, args| {
        let input = text(&args, 1, "oslo.rows.parse")?;
        let pattern = text(&args, 2, "oslo.rows.parse")?;
        match bridge::parse(&input, &pattern) {
            Ok(rows) => ok(rows_value(&rows)),
            Err(why) => failed("oslo.rows.parse", why),
        }
    });

    // oslo.rows.from_json(text) -> rows
    put(rows, "from_json", |_, args| {
        let input = text(&args, 1, "oslo.rows.from_json")?;
        match bridge::from_json(&input) {
            Ok(rows) => ok(rows_value(&rows)),
            Err(why) => failed("oslo.rows.from_json", why),
        }
    });
}

/// `{ "a", "b" }` — the column names a verb selects by.
fn names_of(value: Option<&Value>, owner: &str) -> Result<Vec<String>, LuaError> {
    let Some(Value::Table(table)) = value else {
        return Err(LuaError::new(format!(
            "{owner}: argument #2 must be a list of column names, got {}",
            value.map_or("no value", Value::type_name)
        )));
    };
    let mut names = Vec::new();
    for entry in table.borrow().sequence() {
        match entry {
            Value::Str(name) => names.push(name.to_string()),
            other => {
                return Err(LuaError::new(format!(
                    "{owner}: a column name is a {}, which is not a name",
                    other.type_name()
                )));
            }
        }
    }
    Ok(names)
}

/// The verbs that change a row's columns rather than which rows there are.
///
/// **Bound for the same reason the first eleven were.** A recipe wanting a column dropped or renamed
/// had to write it again in Lua, and a hand-written version of `flatten` or `compact` disagrees with
/// the pipeline's at exactly the edges these verbs exist to get right — a nested record, a `Val::Error`
/// that must survive, a row that is missing the column entirely.
fn reshaping(rows: &mut Table) {
    // oslo.rows.reject(rows, {"a","b"}) -> every column but those
    put(rows, "reject", |_, args| {
        let subject = input(&args, "oslo.rows.reject")?;
        let names = names_of(args.get(1), "oslo.rows.reject")?;
        ok(rows_value(&reshape::reject(&subject, &names)))
    });

    // oslo.rows.rename(rows, from, to) -> the column renamed **in its place**
    put(rows, "rename", |_, args| {
        let subject = input(&args, "oslo.rows.rename")?;
        let from = text(&args, 2, "oslo.rows.rename")?;
        let to = text(&args, 3, "oslo.rows.rename")?;
        ok(rows_value(&reshape::rename(&subject, &from, &to)))
    });

    // oslo.rows.insert / update / upsert (rows, column, expression) -> rows, or nil + why
    //
    // The three differ only in what they refuse, and the refusal is the point: `insert` over a
    // column that exists is nearly always a typo for `update`, and overwriting silently is how a
    // pipeline loses a column without saying so.
    put(rows, "insert", |_, args| {
        assign_from(&args, "oslo.rows.insert", reshape::When::Absent)
    });
    put(rows, "update", |_, args| {
        assign_from(&args, "oslo.rows.update", reshape::When::Present)
    });
    put(rows, "upsert", |_, args| {
        assign_from(&args, "oslo.rows.upsert", reshape::When::Either)
    });

    // oslo.rows.flatten(rows) -> nested records as `outer.inner` columns
    put(rows, "flatten", |_, args| {
        let subject = input(&args, "oslo.rows.flatten")?;
        ok(rows_value(&reshape::flatten(&subject)))
    });

    // oslo.rows.headers(rows) -> the first row becomes the column names
    put(rows, "headers", |_, args| {
        let subject = input(&args, "oslo.rows.headers")?;
        ok(rows_value(&reshape::headers(&subject)))
    });

    // oslo.rows.compact(rows, [column]) -> rows with nothing missing
    //
    // An error cell survives, because it is something: the cell failed and the row is entitled to
    // say so.
    put(rows, "compact", |_, args| {
        let subject = input(&args, "oslo.rows.compact")?;
        let column = opt_text(&args, 2, "oslo.rows.compact")?;
        ok(rows_value(&reshape::compact(&subject, column.as_deref())))
    });

    // oslo.rows.default(rows, column, value) -> the gaps filled, everything else untouched
    put(rows, "default", |_, args| {
        let subject = input(&args, "oslo.rows.default")?;
        let column = text(&args, 2, "oslo.rows.default")?;
        let value = args.get(2).map(from_lua).unwrap_or(Val::Null);
        ok(rows_value(&reshape::default(&subject, &column, &value)))
    });
}

/// The three computing verbs, which differ only in what they refuse.
fn assign_from(args: &[Value], owner: &str, when: reshape::When) -> Result<Vec<Value>, LuaError> {
    let subject = input(args, owner)?;
    let column = text(args, 2, owner)?;
    let expression = text(args, 3, owner)?;
    let (values, problem) = where_::compute(&subject, &expression);
    if let Some(why) = problem {
        return Ok(vec![Value::Nil, Value::str(why)]);
    }
    match reshape::assign(&subject, &column, &values, when) {
        Ok(rows) => ok(rows_value(&rows)),
        Err(why) => failed(owner, why),
    }
}

/// The verbs that choose which rows there are, by position rather than by a test.
fn positional(rows: &mut Table) {
    // oslo.rows.skip(rows, n) / every(rows, n) / enumerate(rows) / reverse(rows)
    put(rows, "skip", |_, args| {
        let subject = input(&args, "oslo.rows.skip")?;
        let n = count(&args, 2, "oslo.rows.skip")?;
        ok(rows_value(&reshape::skip(&subject, n)))
    });
    put(rows, "every", |_, args| {
        let subject = input(&args, "oslo.rows.every")?;
        let n = count(&args, 2, "oslo.rows.every")?;
        ok(rows_value(&reshape::every(&subject, n)))
    });
    // The index leads, because it is what you are about to read.
    put(rows, "enumerate", |_, args| {
        let subject = input(&args, "oslo.rows.enumerate")?;
        ok(rows_value(&reshape::enumerate(&subject)))
    });
    put(rows, "reverse", |_, args| {
        let subject = input(&args, "oslo.rows.reverse")?;
        ok(rows_value(&verbs::reverse(&subject)))
    });
}

/// The verbs that need a **second** set of rows.
///
/// At a prompt these take a Lua expression, because a pipeline is a line and has no shape for "and
/// also read this". Here both sides are already values, so the awkwardness is gone: the second
/// argument is simply the other rows.
fn joining(rows: &mut Table) {
    // oslo.rows.lookup(left, right, on, [keep_unmatched]) -> left rows carrying their match
    //
    // Inner by default: a left row with no match does not survive, because quietly keeping it with
    // empty columns makes "did this match?" unanswerable downstream.
    put(rows, "lookup", |_, args| {
        let left = input(&args, "oslo.rows.lookup")?;
        let right = other_rows(&args, 2, "oslo.rows.lookup")?;
        let on = text(&args, 3, "oslo.rows.lookup")?;
        let keep = matches!(args.get(3), Some(Value::Bool(true)));
        ok(rows_value(&second::lookup(&left, &right, &on, keep)))
    });

    // oslo.rows.append(a, b) -> one stream after the other
    put(rows, "append", |_, args| {
        let left = input(&args, "oslo.rows.append")?;
        let right = other_rows(&args, 2, "oslo.rows.append")?;
        ok(rows_value(&second::append(&left, &right)))
    });

    // oslo.rows.merge(a, b) -> paired by position, the right side winning a collision
    put(rows, "merge", |_, args| {
        let left = input(&args, "oslo.rows.merge")?;
        let right = other_rows(&args, 2, "oslo.rows.merge")?;
        ok(rows_value(&second::merge(&left, &right)))
    });
}

/// The verbs that answer *about* a set of rows rather than transforming it.
fn describing(rows: &mut Table) {
    // oslo.rows.describe(rows) -> a row per column: its type, how full it is, how many rows
    put(rows, "describe", |_, args| {
        let subject = input(&args, "oslo.rows.describe")?;
        ok(rows_value(&summarise::describe(&subject)))
    });

    // oslo.rows.histogram(rows, column) -> the distribution, with a bar
    put(rows, "histogram", |_, args| {
        let subject = input(&args, "oslo.rows.histogram")?;
        let field = text(&args, 2, "oslo.rows.histogram")?;
        ok(rows_value(&summarise::histogram(&subject, &field)))
    });

    // oslo.rows.reduce(rows, expression, [from]) -> one row, or nil + why
    put(rows, "reduce", |_, args| {
        let subject = input(&args, "oslo.rows.reduce")?;
        let expression = text(&args, 2, "oslo.rows.reduce")?;
        let from = opt_text(&args, 3, "oslo.rows.reduce")?;
        let (reduced, problem) = where_::reduce(&subject, &expression, from.as_deref());
        match problem {
            Some(why) => Ok(vec![rows_value(&reduced), Value::str(why)]),
            None => ok(rows_value(&reduced)),
        }
    });

    // oslo.rows.each(rows, expression) -> nothing, or the failure
    //
    // The pressure valve: it runs the expression for its side effects and produces no rows, which
    // is exactly what it does as a stage.
    put(rows, "each", |_, args| {
        let subject = input(&args, "oslo.rows.each")?;
        let expression = text(&args, 2, "oslo.rows.each")?;
        match where_::for_each(&subject, &expression) {
            Some(why) => failed("oslo.rows.each", why),
            None => ok(Value::Nil),
        }
    });
}

/// Argument `n` as a second set of rows.
fn other_rows(args: &[Value], n: usize, owner: &str) -> Result<Vec<Record>, LuaError> {
    match args.get(n - 1) {
        Some(value @ Value::Table(_)) => Ok(records_of(value)),
        other => Err(LuaError::new(format!(
            "{owner}: argument #{n} must be a list of rows, got {}",
            other.map_or("no value", Value::type_name)
        ))),
    }
}
