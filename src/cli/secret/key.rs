//! `oslo secret key` — where a store's keys come from.
//!
//! ```sh
//! oslo secret key list
//! oslo secret key init                              # make the default one, explicitly
//! oslo secret key add file ~/.ssh/oslo-identity
//! oslo secret key add command -- pass show oslo/age-identity
//! oslo secret key rm file ~/.ssh/oslo-identity
//! ```
//!
//! A store with several is a store with several places to look, tried in the order they are
//! written, and the ones that run a program are tried last however they are ordered.

use oslo::secrets::{self, KeySource, Store};

use super::fail;

const USAGE: &str = "usage: oslo secret key list|init|add|rm\n\
                     \n\
                     \x20 add file PATH        read the identity out of a file\n\
                     \x20 add command ARG…     run a program; its output is the identity\n\
                     \x20 rm  file PATH\n\
                     \x20 rm  command ARG…\n\
                     \x20 init                 generate the default key file\n\
                     \x20 list";

pub fn run(store: &Store, args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("list" | "ls") | None => {
            show(store);
            0
        }
        #[cfg(feature = "crypt")]
        Some("init") => init(store),
        Some("add") => match source(&args[1..]) {
            Ok(source) => add(store, &source),
            Err(e) => fail(&e),
        },
        Some("rm" | "remove") => match source(&args[1..]) {
            Ok(source) => remove(store, &source),
            Err(e) => fail(&e),
        },
        Some(other) => {
            eprintln!("oslo secret key: {other}: no such subcommand");
            eprintln!("{USAGE}");
            2
        }
    }
}

/// What the store will try, in the order it will try it.
pub fn show(store: &Store) {
    for source in store.keys.iter().filter(|s| !s.is_external()) {
        println!("{}   {}", source.line(), state_of(source));
    }
    for source in store.keys.iter().filter(|s| s.is_external()) {
        println!("{}   {}", source.line(), state_of(source));
    }
}

/// Whether it is there, said in the fewest words that are true.
fn state_of(source: &KeySource) -> &'static str {
    match source {
        KeySource::File(path) if path.exists() => "present",
        KeySource::File(_) => "not there yet",
        KeySource::Command(_) if secrets::key::no_exec() => "skipped: $OSLO_SECRET_NO_EXEC",
        KeySource::Command(_) => "run when the native keys do not open a file",
    }
}

/// `file PATH` or `command ARG…`, with an optional `--` before the program.
fn source(args: &[String]) -> Result<KeySource, String> {
    let words: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|arg| *arg != "--")
        .collect();
    if words.is_empty() {
        return Err("a key is `file PATH` or `command ARG…`".to_string());
    }
    KeySource::parse(&words.join(" "))
}

fn add(store: &Store, source: &KeySource) -> i32 {
    if store.name.starts_with(secrets::PLUGIN) && source.is_external() {
        return fail("a plugin's store may not run a command");
    }
    let mut conf = match secrets::conf::read() {
        Ok(conf) => conf,
        Err(e) => return fail(&e),
    };
    // **Only the `user` store keeps its implied key when the first explicit one arrives.** Its
    // files are already encrypted to that identity and dropping it out of the list would make them
    // unreadable. Any other store is being told what to use instead, which is the whole reason to
    // write a key line for it.
    if store.name == secrets::USER
        && conf.section(&store.name).is_none_or(|s| s.keys.is_empty())
        && let Some(path) = secrets::identity_path()
    {
        conf.add(&store.name, &KeySource::File(path).line());
    }
    conf.add(&store.name, &source.line());
    match conf.write() {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

fn remove(store: &Store, source: &KeySource) -> i32 {
    let mut conf = match secrets::conf::read() {
        Ok(conf) => conf,
        Err(e) => return fail(&e),
    };
    let line = source.line();
    let gone = conf.remove(&store.name, |written| written == line);
    if gone == 0 {
        return fail(&format!("{}: {line} is not one of its keys", store.name));
    }
    match conf.write() {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

/// Make this store's key file now, rather than on the first `set`.
///
/// Only a build with oslo's own age has a key of its own to make; otherwise the key belongs to
/// whatever program the store names.
#[cfg(feature = "crypt")]
fn init(store: &Store) -> i32 {
    let Some(path) = store.key_file() else {
        return fail(&format!("{}: has no key file to make", store.name));
    };
    if path.exists() {
        eprintln!("oslo secret key: {} is already there", path.display());
        return 1;
    }
    match secrets::key::generate(&path) {
        // The public half is printed because it is the half that is *useful*: it goes in somebody
        // else's `recipient add`, and there is nowhere else to read it from.
        Ok(secret) => {
            println!("{}", path.display());
            println!(
                "{}",
                secrets::native::write_public(&secrets::native::public_of(&secret))
            );
            0
        }
        Err(e) => fail(&e),
    }
}
