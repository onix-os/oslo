//! `oslo.rows` — the structured verbs as functions.
//!
//! # What this is for
//!
//! `crates/oslo-shell/src/data/` is in every build and behind no feature, but its verbs exist only
//! as **pipeline stages**: `ps | where 'rss > 1e9' | sort-by rss`. There is no pipeline in
//! `oslo make`, none inside a registered builtin, and none in a completion provider — so a recipe
//! that wanted rows sorted by a column wrote the sort again in Lua and got a different answer.
//! `table.sort` compares `"100"` below `"9"`; the shell's `sort_by` does not.
//!
//! # The one that could have panicked
//!
//! `oslo.rows.where` takes a **Lua** expression and `where_::filter` evaluates it per row through
//! `oslo_luavm::current::handle()` — the engine that is already running. So calling it from Lua is
//! Lua inside Rust inside Lua, and from a registered builtin it is one level deeper again. That
//! works because `oslo-luavm` falls back to a re-entrant path when the arena is already borrowed;
//! if it ever stops, the failure is a panic in somebody's prompt rather than a message they can
//! read. Hence `where_survives_being_called_from_inside_a_builtin`.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

fn lua(source: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("probe.lua");
    std::fs::write(&file, source).expect("write");
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("HOME", dir.path())
        .env("XDG_DATA_HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The rows every case below works on, with a numeric column whose text order differs.
const ROWS: &str = r#"local ROWS = {
  { name = "alpha", size = 100,  kind = "file" },
  { name = "gamma", size = 9,    kind = "dir"  },
  { name = "beta",  size = 2000, kind = "file" },
}
"#;

fn with_rows(body: &str) -> String {
    format!("{ROWS}{body}")
}

#[test]
fn a_filter_keeps_the_rows_its_expression_is_true_for() {
    let said = lua(&with_rows(
        r#"
local big = oslo.rows.where(ROWS, "size > 50")
print("kept=" .. #big)
print("names=" .. big[1].name .. "," .. big[2].name)
"#,
    ));
    assert!(said.contains("kept=2"), "{said}");
    assert!(said.contains("names=alpha,beta"), "{said}");
}

/// **The reason this is not `table.sort`.** A numeric column sorts as numbers.
#[test]
fn sorting_is_numeric_where_a_lua_sort_would_be_textual() {
    let said = lua(&with_rows(
        r#"
local sorted = oslo.rows.sort_by(ROWS, "size")
print("order=" .. sorted[1].name .. "," .. sorted[2].name .. "," .. sorted[3].name)
"#,
    ));
    assert!(
        said.contains("order=gamma,alpha,beta"),
        "sorted as text: 100 before 9\n{said}"
    );
}

#[test]
fn columns_are_selected_in_the_order_asked_for() {
    let said = lua(&with_rows(
        r#"
local cut = oslo.rows.cols(ROWS, { "size", "name" })
local keys = {}
for k in pairs(cut[1]) do keys[#keys+1] = k end
table.sort(keys)
print("keys=" .. table.concat(keys, ","))
"#,
    ));
    assert!(said.contains("keys=name,size"), "kind survived\n{said}");
}

/// **A number, not the one-row table the pipeline verb answers with.** A caller wanting `#rows`
/// should not have to write `oslo.rows.length(r)[1].length`.
#[test]
fn length_is_a_number() {
    let said = lua(&with_rows(
        r#"print("n=" .. oslo.rows.length(ROWS) .. " type=" .. type(oslo.rows.length(ROWS)))"#,
    ));
    assert!(said.contains("n=3 type=number"), "{said}");
}

#[test]
fn first_and_last_take_from_each_end() {
    let said = lua(&with_rows(
        r#"
print("first=" .. oslo.rows.first(ROWS, 1)[1].name)
print("last=" .. oslo.rows.last(ROWS, 1)[1].name)
"#,
    ));
    assert!(said.contains("first=alpha"), "{said}");
    assert!(said.contains("last=beta"), "{said}");
}

#[test]
fn grouping_and_stats_answer_over_a_column() {
    let said = lua(&with_rows(
        r#"
local groups = oslo.rows.group_by(ROWS, "kind")
print("groups=" .. #groups)
local s = oslo.rows.stats(ROWS, "size")[1]
print("max=" .. tostring(s.max) .. " min=" .. tostring(s.min))
"#,
    ));
    assert!(said.contains("groups=2"), "{said}");
    assert!(said.contains("max=2000"), "{said}");
    assert!(said.contains("min=9"), "{said}");
}

#[test]
fn rendering_answers_each_of_the_three_formats() {
    let said = lua(&with_rows(
        r#"
print("json_is_json=" .. tostring(oslo.rows.render(ROWS, "json"):find("^%[") ~= nil))
print("table_has_header=" .. tostring(oslo.rows.render(ROWS, "table"):find("name") ~= nil))
print("text_has_rows=" .. tostring(oslo.rows.render(ROWS, "text"):find("alpha") ~= nil))
local bad, why = oslo.rows.render(ROWS, "yaml")
print("refused=" .. tostring(bad == nil) .. " says=" .. tostring(tostring(why):find("json") ~= nil))
"#,
    ));
    for want in [
        "json_is_json=true",
        "table_has_header=true",
        "text_has_rows=true",
        "refused=true",
    ] {
        assert!(said.contains(want), "{want}\n{said}");
    }
}

#[test]
fn text_can_be_read_back_into_rows() {
    let said = lua(r#"
print("lines=" .. #oslo.rows.lines("a\nb\nc"))
local rows = oslo.rows.from_json('[{"n":1},{"n":2}]')
print("json=" .. #rows .. " first=" .. tostring(rows[1].n))
"#);
    assert!(said.contains("lines=3"), "{said}");
    assert!(said.contains("json=2 first=1"), "{said}");
}

/// A broken filter reports rather than passing everything through — a filter that silently keeps
/// every row is how a pipeline ending in `rm` removes the wrong thing.
#[test]
fn a_broken_expression_drops_rows_and_says_so() {
    let said = lua(&with_rows(
        r#"
local kept, why = oslo.rows.where(ROWS, "nosuch > 1")
print("kept=" .. #kept)
print("reported=" .. tostring(why ~= nil))
"#,
    ));
    assert!(said.contains("kept=0"), "a broken filter kept rows\n{said}");
}

#[test]
fn something_that_is_not_a_list_of_rows_is_refused() {
    let said = lua(r#"
local ok, err = pcall(function() return oslo.rows.sort_by("nope", "x") end)
print("refused=" .. tostring(not ok))
print("says=" .. tostring(tostring(err):find("list of rows") ~= nil))
"#);
    assert!(said.contains("refused=true"), "{said}");
    assert!(said.contains("says=true"), "{said}");
}

// ─────────────────────────────────────────────── the places a pipeline cannot go

/// **The whole point.** A registered builtin holds the shell, so every pipeline and every call that
/// borrows the environment refuses there. These are pure computation and must not.
#[test]
fn the_verbs_work_from_inside_a_builtin() {
    let said = lua(&with_rows(
        r#"
oslo.register_builtin{ name = "probe", run = function(argv, shell)
  local sorted = oslo.rows.sort_by(ROWS, "size")
  print("sorted=" .. sorted[1].name)
  print("len=" .. oslo.rows.length(ROWS))
  print("rendered=" .. tostring(oslo.rows.render(ROWS, "json"):find("alpha") ~= nil))
end }
oslo.proc.exec("probe")
"#,
    ));
    assert!(said.contains("sorted=gamma"), "{said}");
    assert!(said.contains("len=3"), "{said}");
    assert!(said.contains("rendered=true"), "{said}");
}

/// **The one that could have panicked.** `where` evaluates Lua per row through the engine that is
/// already running, and from a builtin that is one frame deeper still.
#[test]
fn where_survives_being_called_from_inside_a_builtin() {
    let said = lua(&with_rows(
        r#"
oslo.register_builtin{ name = "probe", run = function(argv, shell)
  local big = oslo.rows.where(ROWS, "size > 50")
  print("kept=" .. #big .. " first=" .. tostring(big[1] and big[1].name))
end }
oslo.proc.exec("probe")
print("still_here=yes")
"#,
    ));
    assert!(
        said.contains("kept=2 first=alpha"),
        "re-entering the VM from a builtin broke the filter\n{said}"
    );
    assert!(
        said.contains("still_here=yes"),
        "the shell did not survive it\n{said}"
    );
}

/// And nested: a filter whose result is filtered again, inside a builtin.
#[test]
fn a_filter_of_a_filter_is_still_a_filter() {
    let said = lua(&with_rows(
        r#"
oslo.register_builtin{ name = "probe", run = function()
  local once = oslo.rows.where(ROWS, "size > 50")
  local twice = oslo.rows.where(once, "size > 1000")
  print("twice=" .. #twice .. " name=" .. tostring(twice[1] and twice[1].name))
end }
oslo.proc.exec("probe")
"#,
    ));
    assert!(said.contains("twice=1 name=beta"), "{said}");
}
