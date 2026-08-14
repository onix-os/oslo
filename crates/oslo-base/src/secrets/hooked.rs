//! Asking Lua to do a store's crypto, and telling it when a secret is touched.
//!
//! # Base64, and why it is not a choice
//!
//! A Lua string here is `Rc<str>` — UTF-8, not bytes. An age file is neither: it is binary with a
//! text header, and a ciphertext with an invalid sequence in it would not survive the crossing. So
//! everything that crosses this boundary is base64, both ways, in both hooks. `oslo.secret.seal`
//! already speaks base64 for the same reason, so a handler and that function trade the same shape.
//!
//! # What a handler is told, and what it is not
//!
//! The encrypt and decrypt hooks are given the store, the name and the payload, because doing the
//! crypto requires the payload. The two *watching* hooks are given the store, the name and whether
//! it was a read or a write — and never the value, because a hook that logs is the likeliest thing
//! anybody writes on them and a log of secrets is worse than no log.

use crate::hooks::{self, at};

use base64::Engine;

/// Ask Lua to encrypt, and take the first handler that claims the store.
pub fn encrypt(store: &str, name: &str, value: &[u8]) -> Result<Vec<u8>, String> {
    if !hooks::watched(at::SECRET_ENCRYPT) {
        return Err(unreachable(store, "encrypt"));
    }
    let answer = hooks::answer_hook_with(at::SECRET_ENCRYPT, told(store, name, value));
    taken(answer, store, "encrypt")
}

/// Ask Lua to decrypt.
pub fn decrypt(store: &str, name: &str, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    if !hooks::watched(at::SECRET_DECRYPT) {
        return Err(unreachable(store, "decrypt"));
    }
    let answer = hooks::answer_hook_with(at::SECRET_DECRYPT, told(store, name, ciphertext));
    taken(answer, store, "decrypt")
}

/// What a handler is given: which store, which secret, and the payload as base64.
fn told(store: &str, name: &str, payload: &[u8]) -> Vec<oslo_lua::value::Value> {
    use oslo_lua::value::Value;
    vec![
        Value::str(store),
        Value::str(name),
        Value::str(encode(payload)),
    ]
}

/// What comes back, if anything claimed the store.
fn taken(
    answer: Option<oslo_lua::value::Value>,
    store: &str,
    verb: &str,
) -> Result<Vec<u8>, String> {
    use oslo_lua::value::Value;
    match answer {
        Some(Value::Str(text)) => decode(&text)
            .ok_or_else(|| format!("{store}: what on-secret-{verb} answered is not base64")),
        // `nil` is "not mine", and so is a handler that returned nothing at all. Either way no
        // handler claimed this store, which is a refusal rather than a reason to fall back: falling
        // back would encrypt to a key the store was never meant to use.
        _ => Err(format!(
            "{store}: no on-secret-{verb} handler claimed this store"
        )),
    }
}

/// The message a process with no Lua gets, which is the whole point of declaring the mechanism in
/// the file: it names what is wrong and where the answer lives.
fn unreachable(store: &str, verb: &str) -> String {
    format!(
        "{store}: its crypto is `crypto hook`, and nothing is attached to on-secret-{verb} here. \
         A hook needs a shell that has read your config; `oslo secret` from a script, a Makefile \
         or cron never does"
    )
}

/// The same, asked before anything is touched, so the reason given is the real one.
pub fn unreachable_here(store: &str) -> Option<String> {
    match hooks::watched(at::SECRET_DECRYPT) || hooks::watched(at::SECRET_ENCRYPT) {
        true => None,
        false => Some(unreachable(store, "decrypt")),
    }
}

/// Tell anything watching that a secret is about to be, or has just been, touched.
pub fn touched(before: bool, store: &str, name: &str, writing: bool) {
    let told = [
        ("store", store),
        ("name", name),
        ("how", if writing { "write" } else { "read" }),
    ];
    // Spelled out rather than picking the index into one call, so each dispatch sits beside the
    // moment it reaches — which is what `tests/hook_dispatch_tests.rs` reads to check the two agree.
    match before {
        true if hooks::watched(at::PRE_SECRET_ACCESS) => {
            hooks::fire_at_here(at::PRE_SECRET_ACCESS, &told);
        }
        false if hooks::watched(at::POST_SECRET_ACCESS) => {
            hooks::fire_at_here(at::POST_SECRET_ACCESS, &told);
        }
        _ => {}
    }
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .ok()
}
