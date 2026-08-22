//! `oslo.word` — the two word-level rules the shell applies before a command runs.
//!
//! ```lua
//! oslo.word.braces("src/{a,b}.rs")   --> { "src/a.rs", "src/b.rs" }
//! oslo.word.matches("main.rs", "*.rs")  --> true
//! ```
//!
//! # Why a config needs these at all
//!
//! Both are things a config *already* does, badly, by hand. Brace expansion written in Lua is a
//! recursive split that gets nesting and `{1..9}` wrong; glob matching written in Lua becomes a
//! `string.find` with the pattern's `*` turned into `.*`, which is a different language — `.` is a
//! metacharacter in Lua patterns and is not one in a shell glob, so `matches("a.txt", "a?txt")`
//! quietly answers the wrong thing.
//!
//! The shell has both, tested, and they are pure functions of a string. This is the door.
//!
//! # Pure, which is the point
//!
//! Neither takes an `Environment`, so neither goes through `borrow_env` — they work inside a
//! registered builtin and an answering hook, where `oslo.run` raises. That is most of why they are
//! worth binding: a builtin handed a word has no other way to ask these questions.
//!
//! # One word, not one line
//!
//! [`oslo_base::brace::expand_braces_in_line`] exists beside the function used here and is
//! deliberately not bound. Its callers are alias bodies and `declare -a` literals — places holding
//! a whole line of words. A Lua caller has a word, and offering both spellings would mostly serve
//! to let somebody pick the wrong one.

use super::util::{list, ok, put, text};
use oslo_base::value::{Table, Value};
use oslo_shell::expand::glob::ShellPattern;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Build the `oslo.word` table.
pub fn build() -> Value {
    let mut word = Table::new();
    braces(&mut word);
    matches(&mut word);
    Value::table(word)
}

fn braces(word: &mut Table) {
    // oslo.word.braces(word) -> { "one", "two", … }
    //
    // Always at least one entry: a word with no group is a list of itself, which is what the shell
    // does and what makes the result safe to iterate without checking.
    put(word, "braces", |_, args| {
        let text = text(&args, 1, "oslo.word.braces")?;
        ok(list(
            oslo_base::brace::expand_braces_text(&text)
                .into_iter()
                .map(Value::str),
        ))
    });
}

fn matches(word: &mut Table) {
    // oslo.word.matches(text, pattern) -> boolean
    //
    // **Compiled once and kept**, the same bargain `oslo.re` strikes and for the same measured
    // reason: the caller is filtering a list, so the pattern is constant and the subject is not.
    // Recompiling per candidate turns a linear walk into a quadratic one.
    let cache: Rc<RefCell<HashMap<String, Rc<ShellPattern>>>> =
        Rc::new(RefCell::new(HashMap::new()));
    put(word, "matches", move |_, args| {
        let subject = text(&args, 1, "oslo.word.matches")?;
        let pattern = text(&args, 2, "oslo.word.matches")?;
        let compiled = cache
            .borrow_mut()
            .entry(pattern.clone())
            .or_insert_with(|| Rc::new(ShellPattern::from_unquoted(&pattern)))
            .clone();
        ok(Value::Bool(compiled.matches(&subject)))
    });
}

#[cfg(test)]
#[path = "word/tests.rs"]
mod tests;
