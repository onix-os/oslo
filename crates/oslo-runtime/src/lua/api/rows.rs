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

use super::tool::{records_of, rows_value};
use super::util::{failed, int, ok, opt_text, put, text};
use oslo_base::value::{LuaError, Table, Value};
use oslo_shell::data::Record;
use oslo_shell::data::tools::{bridge, summarise, verbs, where_};

/// Build `oslo.rows`.
pub fn build() -> Value {
    let mut rows = Table::new();
    shaping(&mut rows);
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

    // oslo.rows.sort_by(rows, column) -> sorted
    //
    // The shell's ordering, which is why this is not `table.sort`: a numeric column sorts as numbers
    // and a size sorts by bytes, where a Lua comparison on the rendered text puts "10" below "9".
    put(rows, "sort_by", |_, args| {
        let subject = input(&args, "oslo.rows.sort_by")?;
        let name = text(&args, 2, "oslo.rows.sort_by")?;
        ok(rows_value(&verbs::sort_by(&subject, &name)))
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
