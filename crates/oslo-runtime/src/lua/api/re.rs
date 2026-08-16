//! `oslo.re` — real regular expressions, alongside Lua's own patterns.
//!
//! Lua patterns are not regular expressions: no alternation, no grouping quantifiers, no
//! backtracking-free guarantees, and `%` where everyone else writes `\`. They stay available
//! because they are part of Lua and every Lua library uses them. This is the other one — the
//! `regex` crate oslo already carries for `[[ str =~ ere ]]`, so the shell and the scripting
//! language agree on what a regex means.
//!
//! Compiled patterns are cached, because the natural way to write a loop is
//! `for _, line in ipairs(lines) do if oslo.re.match(line, p) then … end end`, and recompiling a
//! pattern per line is the difference between instant and noticeable.

use super::util::{list, ok, put, record, text};
use oslo_base::value::{LuaError, LuaResult};
use oslo_base::value::{Table, Value};
use regex::Regex;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn build() -> Value {
    let mut re = Table::new();
    let cache: Cache = Rc::new(RefCell::new(HashMap::new()));

    // oslo.re.match(text, pattern) -> {match=…, start=…, stop=…, groups={…}} or nil
    let for_match = Rc::clone(&cache);
    put(&mut re, "match", move |_, args| {
        let subject = text(&args, 1, "oslo.re.match")?;
        let pattern = text(&args, 2, "oslo.re.match")?;
        let compiled = compile(&for_match, &pattern)?;
        ok(match compiled.captures(&subject) {
            Some(caps) => describe(&caps),
            None => Value::Nil,
        })
    });

    // oslo.re.find_all(text, pattern) -> a table of the same records, one per match
    let for_all = Rc::clone(&cache);
    put(&mut re, "find_all", move |_, args| {
        let subject = text(&args, 1, "oslo.re.find_all")?;
        let pattern = text(&args, 2, "oslo.re.find_all")?;
        let compiled = compile(&for_all, &pattern)?;
        ok(list(compiled.captures_iter(&subject).map(|c| describe(&c))))
    });

    // oslo.re.test(text, pattern) -> boolean
    //
    // Separate from `match` because "does this match" is most of the uses, and a caller writing
    // `if oslo.re.match(…) then` has to know that a *successful* match of an empty string is a
    // table and therefore truthy — which is easy to get wrong the other way round.
    let for_test = Rc::clone(&cache);
    put(&mut re, "test", move |_, args| {
        let subject = text(&args, 1, "oslo.re.test")?;
        let pattern = text(&args, 2, "oslo.re.test")?;
        let compiled = compile(&for_test, &pattern)?;
        ok(Value::Bool(compiled.is_match(&subject)))
    });

    // oslo.re.replace(text, pattern, replacement, count)
    //
    // `$1` and `${name}` in the replacement, which is the `regex` crate's own syntax rather than
    // Lua's `%1`. Mixing the two would mean a pattern in one dialect and a replacement in
    // another; `string.gsub` is right there for the Lua spelling.
    let for_replace = Rc::clone(&cache);
    put(&mut re, "replace", move |_, args| {
        let subject = text(&args, 1, "oslo.re.replace")?;
        let pattern = text(&args, 2, "oslo.re.replace")?;
        let replacement = text(&args, 3, "oslo.re.replace")?;
        let compiled = compile(&for_replace, &pattern)?;
        let limit = args
            .get(3)
            .and_then(Value::as_number)
            .and_then(|n| n.as_int());
        ok(Value::str(match limit {
            Some(n) if n >= 0 => compiled
                .replacen(&subject, n as usize, replacement.as_str())
                .into_owned(),
            _ => compiled
                .replace_all(&subject, replacement.as_str())
                .into_owned(),
        }))
    });

    // oslo.re.split(text, pattern) -> the pieces between matches
    let for_split = Rc::clone(&cache);
    put(&mut re, "split", move |_, args| {
        let subject = text(&args, 1, "oslo.re.split")?;
        let pattern = text(&args, 2, "oslo.re.split")?;
        let compiled = compile(&for_split, &pattern)?;
        ok(list(compiled.split(&subject).map(Value::str)))
    });

    // oslo.re.quote(text) -> a pattern matching exactly that text
    put(&mut re, "quote", |_, args| {
        let subject = text(&args, 1, "oslo.re.quote")?;
        ok(Value::str(regex::escape(&subject)))
    });

    Value::table(re)
}

/// Compiled patterns, keyed by their source.
type Cache = Rc<RefCell<HashMap<String, Regex>>>;

fn compile(cache: &Cache, pattern: &str) -> LuaResult<Regex> {
    if let Some(compiled) = cache.borrow().get(pattern) {
        return Ok(compiled.clone());
    }
    // A pattern that does not compile is a mistake in the script, not a condition to handle — so
    // this raises where the rest of `oslo.*` answers `nil, message`. The message is the `regex`
    // crate's own, which points at the character it choked on.
    let compiled =
        Regex::new(pattern).map_err(|e| LuaError::new(format!("oslo.re: invalid pattern: {e}")))?;
    // Unbounded, but bounded in practice: the key is the pattern *source*, and a script has a
    // fixed number of those written in it. A program generating fresh patterns in a loop would
    // grow this, and would also be doing something unusual enough to notice.
    cache
        .borrow_mut()
        .insert(pattern.to_string(), compiled.clone());
    Ok(compiled)
}

/// One match, as the record every function here hands back.
fn describe(caps: &regex::Captures) -> Value {
    let whole = caps.get(0).expect("group 0 always participates");
    // 1-based and inclusive, matching `string.find` — a Lua script should not have to remember
    // which of its two regex APIs counts from zero.
    let mut groups = Vec::new();
    for group in caps.iter().skip(1) {
        groups.push(match group {
            Some(m) => Value::str(m.as_str()),
            // A group that did not participate is `false`, not nil: nil in a sequence is a hole,
            // so `#groups` would stop counting at the first optional group that missed.
            None => Value::Bool(false),
        });
    }
    record(vec![
        ("match", Value::str(whole.as_str())),
        ("start", Value::int(whole.start() as i64 + 1)),
        ("stop", Value::int(whole.end() as i64)),
        ("groups", list(groups)),
    ])
}
