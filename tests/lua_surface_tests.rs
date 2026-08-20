//! Every name Lua 5.4 defines is *there*, even the ones oslo does not implement.
//!
//! # The rule this enforces
//!
//! **Nothing on the standard surface is `nil`.** Left `nil`, the first use of a name is `attempt
//! to call a nil value` and the reader goes looking for a typo that is not there; present, it
//! either works or says why it will not.
//!
//! Twenty names were `nil` — among them `io.stderr`, which is how a Lua program complains, and
//! `table.move`, which cannot be written in Lua without getting the overlapping case wrong. The
//! rule was stated in two places and checked in none, which is the only reason twenty could
//! accumulate.
//!
//! **The VM has since grown most of what it once refused**, so the second half of the rule is no
//! longer "unimplemented names error by name" — there are none left. What remains is oslo's own
//! three refusals, which are deliberate and are checked below. `os.setlocale` was the last name
//! still missing, and it is answered rather than refused: see
//! `crates/oslo-runtime/src/lua/api/policy.rs`.
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
///
/// **This case used to name `string.pack` and `coroutine.wrap`**, which the VM has since grown; it
/// was asserting that two working functions were broken, and failed the moment they started
/// working. What it was really guarding is that a name a reader cannot use says which name it was,
/// and the names that do that now are oslo's three deliberate refusals — see
/// `crates/oslo-runtime/src/lua/api/policy.rs`. `os.execute` and `io.popen` shared one sentence
/// that named neither of them until this test was pointed at them.
#[test]
fn a_refused_name_says_what_it_is() {
    let printed = lua("for _, probe in ipairs{\n\
         \x20 {'os.execute', function() return os.execute('true') end},\n\
         \x20 {'io.popen', function() return io.popen('true') end},\n\
         \x20 {'os.tmpname', function() return os.tmpname() end},\n\
         } do\n\
         \x20 local ok, err = pcall(probe[2])\n\
         \x20 print(probe[1] .. ' | ' .. tostring(ok) .. ' | ' .. tostring(err))\n\
         end\n");
    for name in ["os.execute", "io.popen", "os.tmpname"] {
        let line = printed
            .lines()
            .find(|line| line.starts_with(name))
            .unwrap_or_else(|| panic!("{name} did not run: {printed}"));
        assert!(line.contains("| false |"), "{name} did not refuse: {line}");
        // The refusal has to carry the name, or a traceback through a line that calls two of them
        // says which problem there is and not which call has it.
        assert!(
            line.split('|').nth(2).is_some_and(|why| why.contains(name)),
            "{name} refused without naming itself: {line}"
        );
    }
}

/// **`os.setlocale` was the one standard name that was `nil`**, and it is answerable rather than
/// refusable: this binary is static and musl-linked, so `C` is not just the current locale, it is
/// the only one. The answers are Lua's own contract — `nil` for a locale that cannot be honoured.
#[test]
fn setlocale_answers_for_the_only_locale_there_is() {
    let printed = lua("print('ask', os.setlocale())\n\
         print('c', os.setlocale('C'))\n\
         print('posix', os.setlocale('POSIX'))\n\
         print('native', os.setlocale(''))\n\
         print('other', tostring(os.setlocale('en_US.UTF-8')))\n\
         print('category', os.setlocale('C', 'time'))\n\
         local ok, err = pcall(function() return os.setlocale('C', 'bogus') end)\n\
         print('badcat', tostring(ok), tostring(err))\n");
    for wanted in [
        "ask\tC",
        "c\tC",
        "posix\tC",
        "native\tC",
        "other\tnil",
        "category\tC",
    ] {
        assert!(printed.contains(wanted), "wanted {wanted:?} in: {printed}");
    }
    assert!(
        printed.contains("invalid option 'bogus'"),
        "a category nobody defines should be reported: {printed}"
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

/// **`_VERSION` is the language's answer, not the VM's.**
///
/// luna reports `"luna"`. The tree walker it replaced reported `"Lua 5.4"`, and the migration
/// dropped the line — so every script asking the standard question got a name no Lua has ever been
/// written against, and the release smoke test failed on it from that commit onward without anyone
/// reading the log. oslo speaks Lua 5.4 and `docs/features/lua-interpreter.md` opens by saying so.
#[test]
fn the_version_is_the_language_not_the_vm() {
    assert_eq!(lua("print(_VERSION)").trim(), "Lua 5.4");
    // The shape the smoke test asks for, so the two cannot drift apart.
    assert!(lua(r#"print("lua " .. _VERSION)"#).contains("lua Lua 5.4"));
}
