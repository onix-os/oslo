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

/// **Every verb that transforms rows is reachable as a function**, and the list is not maintained by
/// hand on both sides.
///
/// The module doc's argument for binding these is that a recipe wanting rows sorted by a column
/// otherwise writes the sort again in Lua and gets a different answer. That argument does not stop
/// at `sort_by`: it applies to every verb, and for a long time only eleven of the forty were here
/// while the page claimed `oslo.rows` was "the same verbs as functions". Twenty-one were not.
///
/// The producers and bridges are excluded by name rather than by a rule, because each is excluded
/// for its own reason: `ls`, `ps` and `df` read the machine rather than rows; `lines`, `parse` and
/// `from` take *text*, and are bound under their own names where they belong; `to` is `render`;
/// `detect-columns` guesses at a layout, which is a thing to do to a document rather than to rows.
#[test]
fn every_row_verb_is_also_a_function() {
    // What a verb here would have nothing to transform, and why.
    const NOT_ROW_TO_ROW: &[&str] = &[
        "df",
        "ps",
        "ls", // read the machine, not rows
        // A producer too, and one Lua already reaches by another road: `oslo.history.commands`
        // answers the same folded rows, so binding it here would be two names for one thing.
        "history",
        "lines",
        "parse",
        "from",
        "detect-columns", // take text; bound under their own names
        // **Scalar verbs, and Lua is already a language with strings in it.** `text` and `path`
        // read their operands when nothing upstream had rows, which is what lets them open a
        // pipeline — and it is also what they would have nothing to do without: a Lua caller who
        // wants a string split writes `s:gmatch`, and binding a second spelling of that here would
        // be inventing a disagreement about how splitting works.
        "text",
        "path",
        "to",             // is `render`
        // **A viewer, not a transformation.** `explore` takes the screen, waits for a person and
        // answers nothing — there is no value for `oslo.rows.explore(rows)` to be, and a script
        // that blocked on a keypress in the middle of a `map` would be the worst thing in the
        // Lua API. The other verbs here have somewhere else to be reached from; this one has
        // nowhere to be reached from at all, which is the point of it.
        "explore",
    ];

    let registered = include_str!("../crates/oslo-shell/src/data/tools/registry.rs");
    let verbs: Vec<String> = registered
        .split('"')
        .filter(|word| {
            !word.is_empty()
                && word.chars().all(|c| c.is_ascii_lowercase() || c == '-')
                && !NOT_ROW_TO_ROW.contains(word)
        })
        .map(|word| word.replace('-', "_"))
        .collect();
    assert!(verbs.len() > 20, "the verb list did not parse: {verbs:?}");

    let names: Vec<String> = verbs.iter().map(|v| format!("\"{v}\"")).collect();
    let probe = format!(
        "local missing = {{}}
         for _, n in ipairs({{{}}}) do
           if type(oslo.rows[n]) ~= 'function' then missing[#missing+1] = n end
         end
         print(table.concat(missing, ' '))",
        names.join(",")
    );

    let answered = lua(&probe);
    assert!(
        answered.trim().is_empty(),
        "these verbs have no oslo.rows function: {}",
        answered.trim()
    );
}

/// **The three computing verbs differ only in what they refuse, and the refusal is the point.**
/// `insert` over a column that exists is nearly always a typo for `update`; overwriting silently is
/// how a caller loses a column without being told. The wording is the verb's own, so a person who
/// has seen it at a prompt reads the same sentence here.
#[test]
fn the_computing_verbs_refuse_here_exactly_as_they_do_in_a_pipeline() {
    let said = lua(&with_rows(
        r#"
local ok = oslo.rows.insert(ROWS, "kb", "size // 1024")
print("insert=" .. #ok .. "," .. tostring(ok[1].kb))

local nope, why = oslo.rows.insert(ROWS, "size", "1")
print("dup=" .. tostring(nope) .. "|" .. tostring(why))

local gone, why2 = oslo.rows.update(ROWS, "absent", "1")
print("missing=" .. tostring(gone) .. "|" .. tostring(why2))

-- upsert refuses neither, which is the whole of what makes it upsert.
print("upsert_new=" .. tostring(oslo.rows.upsert(ROWS, "fresh", "1")[1].fresh))
print("upsert_old=" .. tostring(oslo.rows.upsert(ROWS, "size", "0")[1].size))
"#,
    ));
    assert!(said.contains("insert=3,"), "{said}");
    assert!(said.contains("dup=nil|"), "{said}");
    assert!(said.contains("already a column"), "{said}");
    assert!(said.contains("missing=nil|"), "{said}");
    assert!(said.contains("no such column"), "{said}");
    assert!(said.contains("upsert_new=1"), "{said}");
    assert!(said.contains("upsert_old=0"), "{said}");
}

/// **The verbs that need a second set of rows are easier here than at a prompt.** A pipeline is a
/// line and has no shape for "and also read this", so `lookup` takes a Lua expression there. Here
/// both sides are already values, so the second argument is simply the other rows.
#[test]
fn the_joining_verbs_take_the_other_rows_as_an_argument() {
    let said = lua(&with_rows(
        r#"
local other = { { name = "alpha", city = "X" } }
print("lookup=" .. #oslo.rows.lookup(ROWS, other, "name"))
print("kept=" .. #oslo.rows.lookup(ROWS, other, "name", true))
print("append=" .. #oslo.rows.append(ROWS, other))
print("merge=" .. tostring(oslo.rows.merge({{a=1}}, {{b=2}})[1].b))
"#,
    ));
    // Inner by default: only the row that matched.
    assert!(said.contains("lookup=1"), "{said}");
    // Left-outer when asked, which is how you find what did *not* match.
    assert!(said.contains("kept=3"), "{said}");
    assert!(said.contains("append=4"), "{said}");
    assert!(said.contains("merge=2"), "{said}");
}

/// The reshaping verbs answer what their pipeline stages answer, including the awkward ones: a
/// nested record flattens to the name a path would reach it by, and `headers` turns the first row
/// into the column names rather than leaving it as data.
#[test]
fn the_reshaping_verbs_answer_what_their_stages_do() {
    let said = lua(&with_rows(
        r#"
print("reject=" .. tostring(oslo.rows.reject(ROWS, {"size"})[1].size))
print("rename=" .. tostring(oslo.rows.rename(ROWS, "name", "who")[1].who))
print("flatten=" .. tostring(oslo.rows.flatten({{ state = { pid = 9 } }})[1]["state.pid"]))
print("headers=" .. tostring(oslo.rows.headers({{a="N"},{a="v"}})[1].N))
print("skip=" .. #oslo.rows.skip(ROWS, 1) .. " every=" .. #oslo.rows.every(ROWS, 2))
print("enumerate=" .. tostring(oslo.rows.enumerate(ROWS)[1].index))
print("reverse=" .. oslo.rows.reverse(ROWS)[1].name)
print("final=" .. oslo.rows.final(ROWS, 1)[1].name)
print("default=" .. tostring(oslo.rows.default({{a=1},{b=2}}, "a", 9)[2].a))
"#,
    ));
    assert!(said.contains("reject=nil"), "{said}");
    assert!(said.contains("rename=alpha"), "{said}");
    assert!(said.contains("flatten=9"), "the path's own name: {said}");
    assert!(said.contains("headers=v"), "{said}");
    assert!(said.contains("skip=2 every=2"), "{said}");
    assert!(said.contains("enumerate=0"), "{said}");
    // The fixture's order is alpha, gamma, beta — so the last row is `beta`, both ways round.
    assert!(said.contains("reverse=beta"), "{said}");
    assert!(said.contains("final=beta"), "{said}");
    assert!(said.contains("default=9"), "{said}");
}

/// **The four kinds Lua could not make.** A size, a duration and a time reach Lua as plain numbers
/// so that `free < 1e9` is arithmetic — and a number handed back cannot say which it was, so every
/// Lua-valued verb flattened them. An error could not be written at all, and one handed *in* came
/// back as a record of one field: a failure turned into data.
#[test]
fn a_lua_caller_can_build_the_kinds_that_draw() {
    let said = lua(r#"
print("size="     .. oslo.rows.render({{ s = oslo.rows.size(4509715660) }}, "table"))
print("duration=" .. oslo.rows.render({{ d = oslo.rows.duration(1500000000) }}, "table"))
print("fail="     .. oslo.rows.render({{ e = oslo.rows.fail("stale handle") }}, "table"))
-- and the value is still the number underneath, so it sorts and compares as one
print("transport=" .. oslo.rows.render({{ s = oslo.rows.size(2048) }}, "text"))
"#);
    assert!(said.contains("4.2G"), "a size draws humanly: {said}");
    assert!(said.contains("1.5s"), "a duration draws humanly: {said}");
    assert!(
        said.contains("<error: stale handle>"),
        "an error is a failed cell, not a record: {said}"
    );
    assert!(
        said.contains("2048"),
        "and a program still gets the number: {said}"
    );
}

/// **`render` could write a delimited document and nothing could read one back.** `from_json` had
/// both halves; the delimited pair had only the writer, for no reason anyone wrote down.
#[test]
fn a_delimited_document_can_be_read_as_well_as_written() {
    let said = lua(r#"
local doc = 'name,note\nann,"one\ntwo"\nbob,plain\n'
local r = oslo.rows.from_csv(doc)
print("rows=" .. #r)
print("quoted=" .. (r[1].note:gsub("\n", "/")))
print("round=" .. #oslo.rows.from_csv(oslo.rows.render(r, "csv")))
print("tsv=" .. #oslo.rows.from_tsv("a\tb\n1\t2\n"))
local bad, why = oslo.rows.from_csv('a\n"never closed\n')
print("bad=" .. tostring(bad) .. "|" .. tostring(why))
"#);
    assert!(said.contains("rows=2"), "{said}");
    // The newline inside the quoted field is data, not a record boundary.
    assert!(said.contains("quoted=one/two"), "{said}");
    assert!(said.contains("round=2"), "{said}");
    assert!(said.contains("tsv=1"), "{said}");
    assert!(said.contains("bad=nil|"), "{said}");
    assert!(said.contains("never closed"), "{said}");
}
