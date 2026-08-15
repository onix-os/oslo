//! Every name Lua 5.4 defines is *there*, even the ones oslo does not implement.
//!
//! # The rule this enforces
//!
//! `stdlib/mod.rs` and `docs/features/lua-interpreter.md` both state it: **everything unimplemented
//! is present and erroring, never `nil`.** `coroutine.wrap` raises "not implemented in oslo's Lua"
//! and names the file and line; left `nil` the first use is `attempt to index a nil value`, and the
//! reader goes looking for a typo that is not there.
//!
//! Twenty names were `nil` — among them `io.stderr`, which is how a Lua program complains, and
//! `table.move`, which cannot be written in Lua without getting the overlapping case wrong. The
//! rule was stated in two places and checked in none, which is the only reason twenty could
//! accumulate.
//!
//! So this walks the standard surface and asks. A name added to Lua's library and forgotten here
//! fails the walk rather than waiting for somebody's script to find it.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

/// Run a Lua chunk through the real binary and answer with everything it printed.
fn lua(source: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("probe.lua");
    std::fs::write(&file, source).expect("write");
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("HOME", dir.path())
        .env("XDG_DATA_HOME", dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The whole of Lua 5.4's standard surface, by name.
const SURFACE: &str = r#"
local missing = {}
local function chk(name, v) if v == nil then missing[#missing + 1] = name end end
for _, n in ipairs{"assert","collectgarbage","dofile","error","getmetatable","ipairs","load",
  "next","pairs","pcall","print","rawequal","rawget","rawlen","rawset","require","select",
  "setmetatable","tonumber","tostring","type","warn","xpcall","_G","_VERSION",
  "coroutine","debug","io","math","os","package","string","table","utf8"} do chk(n, _G[n]) end
local libs = {
  string = {"byte","char","dump","find","format","gmatch","gsub","len","lower","match","pack",
            "packsize","rep","reverse","sub","unpack","upper"},
  table  = {"concat","insert","move","pack","remove","sort","unpack"},
  math   = {"abs","ceil","cos","deg","exp","floor","fmod","huge","log","max","maxinteger","min",
            "mininteger","modf","pi","rad","random","randomseed","sin","sqrt","tan","tointeger",
            "type","ult"},
  os     = {"clock","date","difftime","execute","exit","getenv","remove","rename","setlocale",
            "time","tmpname"},
  io     = {"close","flush","input","lines","open","output","popen","read","stderr","stdin",
            "stdout","tmpfile","type","write"},
  coroutine = {"close","create","isyieldable","resume","running","status","wrap","yield"},
  utf8   = {"char","charpattern","codepoint","codes","len","offset"},
  package = {"config","cpath","loaded","path","preload","searchers","searchpath"},
}
for lib, fields in pairs(libs) do
  local t = _G[lib]
  if t == nil then missing[#missing + 1] = lib
  else for _, f in ipairs(fields) do chk(lib .. "." .. f, t[f]) end end
end
table.sort(missing)
print("MISSING:" .. table.concat(missing, " "))
"#;

/// **Nothing standard is `nil`.**
#[test]
fn every_standard_name_exists() {
    let printed = lua(SURFACE);
    let line = printed
        .lines()
        .find(|l| l.starts_with("MISSING:"))
        .unwrap_or_else(|| panic!("the probe did not run: {printed}"));
    assert_eq!(
        line, "MISSING:",
        "these are nil, where the rule says they must be present and erroring"
    );
}

/// **And the ones that refuse, refuse by name** — which is the half of the rule that makes it
/// useful rather than merely tidy.
#[test]
fn an_unimplemented_name_says_what_it_is() {
    let printed = lua(
        "local ok, err = pcall(function() return string.pack('i4', 1) end)\nprint(err)\n\
         local ok2, err2 = pcall(function() return coroutine.wrap(print) end)\nprint(err2)\n",
    );
    assert!(
        printed.contains("string.pack is not implemented in oslo's Lua"),
        "{printed}"
    );
    assert!(
        printed.contains("coroutine.wrap is not implemented in oslo's Lua"),
        "{printed}"
    );
}

/// `io.stderr:write` is how a Lua program complains, and it was `attempt to index a nil value`.
#[test]
fn the_standard_streams_can_be_written_to() {
    let printed = lua(
        "io.stderr:write('to-stderr\\n')\nio.stdout:write('to-stdout\\n')\n\
         print('type', io.type(io.stderr), io.type('x'))\n",
    );
    assert!(printed.contains("to-stderr"), "{printed}");
    assert!(printed.contains("to-stdout"), "{printed}");
    assert!(printed.contains("type\tfile\tnil"), "{printed}");
}

/// `table.move` is the one that cannot be written in Lua without getting the overlap wrong.
#[test]
fn table_move_handles_an_overlapping_range() {
    let printed = lua(
        "print(table.concat(table.move({1,2,3,4,5}, 1, 3, 3), ','))\n\
         print(table.concat(table.move({1,2,3,4,5}, 3, 5, 1), ','))\n\
         print(table.concat(table.move({7,8}, 1, 2, 1, {}), ','))\n",
    );
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(lines[0], "1,2,1,2,3", "moving up over itself: {printed}");
    assert_eq!(lines[1], "3,4,5,4,5", "moving down over itself: {printed}");
    assert_eq!(lines[2], "7,8", "moving into another table: {printed}");
}
