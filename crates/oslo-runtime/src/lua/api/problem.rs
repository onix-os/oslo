//! What the second half of `nil, message` is: an object that still *is* the message.
//!
//! ```lua
//! local text, err = oslo.fs.read("/nope")
//! print(err)                        -- /nope: No such file or directory
//! print(err.kind, err.code)         -- not-found  2
//! if err.kind == "permission" then … end
//! ```
//!
//! # Why not a string
//!
//! A caller who wants to know *what went wrong* — as opposed to showing somebody — had one option,
//! which was to match English. `err:find("No such file")` breaks when the C library is translated,
//! and it cannot tell "the directory is not readable" from "the file is not there" without knowing
//! both messages. The kind and the errno are the facts; the sentence is a rendering of them.
//!
//! # Why it is still a string in every way that was already working
//!
//! `__tostring` answers the same sentence the old string was, `__concat` makes `"oops: " .. err`
//! work, and `__index` falls through to the string library — so `err:find("nope")`, `err:match(…)`
//! and `err:upper()` all still do what they did. **Nothing that read the message has to change**,
//! which is the only way to add structure to a convention this widely used.
//!
//! The fallthrough is compiled Lua rather than Rust: it has to reach `string`, which is the VM's
//! table, and a native reaching back into the VM to index a global on every miss would be the
//! expensive way to write `string[key]`.

use super::util::native;
use oslo_base::value::{Table, Value};
use oslo_luavm::Host;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    /// The metatable every failure wears, built once.
    static META: RefCell<Option<Rc<RefCell<Table>>>> = const { RefCell::new(None) };
}

/// `__index`, in the language that can see `string`. See the module note.
///
/// `rawget` rather than a plain index, because indexing the error inside its own `__index` is how
/// that becomes infinite recursion.
const FALLTHROUGH: &str = r#"return function(err, key)
    local method = string[key]
    if method == nil then return nil end
    return function(_, ...) return method(rawget(err, "message"), ...) end
end"#;

/// Build the shared metatable. Called once, when the `oslo` table is assembled.
pub(super) fn install(host: &dyn Host) {
    let mut meta = Table::new();
    meta.set_str("__name", Value::str("oslo.failure"));
    meta.set_str(
        "__tostring",
        native("__tostring", |_, args| {
            Ok(vec![Value::str(message_of(args.first()))])
        }),
    );
    // Either side may be the failure — `err .. "!"` and `"oops: " .. err` both reach here.
    meta.set_str(
        "__concat",
        native("__concat", |_, args| {
            let left = message_of(args.first());
            let right = message_of(args.get(1));
            Ok(vec![Value::str(format!("{left}{right}"))])
        }),
    );
    if let Ok(values) = host.eval(FALLTHROUGH, "=oslo.failure")
        && let Some(index @ Value::Function(_)) = values.into_iter().next()
    {
        meta.set_str("__index", index);
    }
    META.with(|slot| *slot.borrow_mut() = Some(Rc::new(RefCell::new(meta))));
}

/// A value as the text it stands for: the message for a failure, `tostring` for anything else.
fn message_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::Table(table)) => match table.borrow().get_str("message") {
            Value::Str(text) => text.to_string(),
            _ => "?".to_string(),
        },
        Some(other) => other.to_display(),
        None => String::new(),
    }
}

/// A failure carrying `message`, plus whatever facts the caller could establish.
pub fn new(message: String, facts: Vec<(&str, Value)>) -> Value {
    let mut table = Table::new();
    table.set_str("message", Value::str(&message));
    for (name, value) in facts {
        table.set_str(name, value);
    }
    table.metatable = META.with(|slot| slot.borrow().clone());
    Value::table(table)
}

/// What an I/O failure was, as a name that does not change when the message does.
///
/// The slugs are the shell's, not `std`'s: a config asking "was that a permission problem" should
/// not have to know Rust's spelling of it, and the set is deliberately small — a name nobody
/// matches on is a name that only makes the answer harder to read.
pub fn kind_of(kind: std::io::ErrorKind) -> &'static str {
    use std::io::ErrorKind as K;
    match kind {
        K::NotFound => "not-found",
        K::PermissionDenied => "permission",
        K::AlreadyExists => "exists",
        K::InvalidInput | K::InvalidData => "invalid",
        K::UnexpectedEof => "truncated",
        K::TimedOut => "timeout",
        K::Interrupted => "interrupted",
        _ => "other",
    }
}
