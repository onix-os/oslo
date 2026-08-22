//! `oslo.ui.match` / `rank` / `wrap` and `oslo.theme.style` — the pure half of the drawing surface.
//!
//! # What these are for
//!
//! A completion provider's `answer` runs inside the line editor, which is holding the shell — so
//! `oslo.run` and every other locked call refuses there. What a provider could do instead was a
//! `string.find`, which answers *whether* but never *how well*, and `max_items` then truncated
//! whatever order the Lua loop happened to build. These are the shell's own scorer, so a config and
//! the built-in finder agree about what `gco` means.

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
        .env_remove("NO_COLOR")
        .env("TERM", "xterm-256color")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The canonical abbreviation, and a non-match answering `nil` rather than zero.
#[test]
fn a_match_scores_and_a_non_match_is_nil() {
    let said = lua(r#"
print("hit=" .. tostring(oslo.ui.match("git checkout", "gco") ~= nil))
print("miss=" .. tostring(oslo.ui.match("git checkout", "zzz")))
"#);
    assert!(said.contains("hit=true"), "{said}");
    assert!(said.contains("miss=nil"), "{said}");
}

/// **The reason `rank` exists.** The order is the scorer's, not the order the table was built in.
#[test]
fn ranking_puts_the_better_candidate_first() {
    let said = lua(r#"
local r = oslo.ui.rank({"libcargo", "cargo"}, "ca")
print("first=" .. r[1].text)
print("count=" .. #r)
"#);
    assert!(
        said.contains("first=cargo"),
        "the input order survived ranking\n{said}"
    );
    assert!(said.contains("count=2"), "{said}");
}

/// Non-matches are dropped, so `#result` is the number worth showing.
#[test]
fn ranking_drops_what_did_not_match() {
    let said = lua(r#"
local r = oslo.ui.rank({"git checkout", "nothing alike", "git commit"}, "gco")
print("count=" .. #r)
"#);
    assert!(said.contains("count=2"), "{said}");
}

#[test]
fn a_limit_cuts_after_the_order_exists() {
    let said = lua(r#"
local r = oslo.ui.rank({"libcargo", "cargo", "cartography"}, "ca", { limit = 1 })
print("count=" .. #r .. " first=" .. r[1].text)
"#);
    assert!(said.contains("count=1"), "{said}");
    assert!(said.contains("first=cargo"), "{said}");
}

/// **1-based, because the next thing the caller writes is `string.sub`.**
#[test]
fn the_offsets_are_ready_for_string_sub() {
    let said = lua(r#"
local at = oslo.ui.match_at("echo", "ec")
print("first=" .. at[1] .. " letter=" .. ("echo"):sub(at[1], at[1]))
"#);
    assert!(said.contains("first=1"), "{said}");
    assert!(
        said.contains("letter=e"),
        "0-based offsets reached Lua\n{said}"
    );
}

#[test]
fn an_unknown_preset_is_refused_and_names_the_four() {
    let said = lua(r#"
local ok, err = pcall(function() oslo.ui.match("a", "b", "fuzzy") end)
print("refused=" .. tostring(not ok))
print("names=" .. tostring(tostring(err):find("loose") ~= nil))
"#);
    assert!(said.contains("refused=true"), "{said}");
    assert!(said.contains("names=true"), "{said}");
}

/// Wrapping counts cells, which is why it is not a `string.gsub` in a config: colour is free.
#[test]
fn wrapping_measures_cells_not_bytes() {
    let said = lua(r#"
local plain = oslo.ui.wrap("the quick brown fox", 9)
print("plain=" .. #plain)
local painted = oslo.ui.wrap("\27[31mthe\27[0m quick brown fox", 9)
print("painted=" .. #painted)
"#);
    assert!(said.contains("plain=2"), "{said}");
    assert!(
        said.contains("painted=2"),
        "escapes were counted as cells\n{said}"
    );
}

#[test]
fn a_defined_style_is_what_style_resolves() {
    let said = lua(r#"
oslo.theme.define("warn", "fg:yellow bold")
local painted = oslo.theme.style("careful", "warn")
print("escaped=" .. tostring(painted:find("\27") ~= nil))
print("kept=" .. tostring(painted:find("careful") ~= nil))
"#);
    assert!(said.contains("escaped=true"), "{said}");
    assert!(said.contains("kept=true"), "{said}");
}

/// **The reason a caller never has to check.** At `NO_COLOR` the answer is the text itself.
#[test]
fn no_colour_returns_the_text_unpainted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("probe.lua");
    std::fs::write(
        &file,
        r#"print("painted=[" .. oslo.theme.style("x", "fg:red bold") .. "]")"#,
    )
    .expect("write");
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("HOME", dir.path())
        .env("XDG_DATA_HOME", dir.path())
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        said.contains("painted=[x]"),
        "NO_COLOR still emitted escapes\n{said:?}"
    );
}
