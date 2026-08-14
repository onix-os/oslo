//! `oslo secret recipient` — who a store encrypts to.
//!
//! ```sh
//! oslo secret recipient                       # the list
//! oslo secret recipient --export > RECIPIENTS  # to hand to somebody
//! oslo secret recipient add age1ql3z7…
//! oslo secret recipient add --from RECIPIENTS
//! oslo secret recipient rm age1ql3z7…
//! ```
//!
//! **Adding one does not re-encrypt anything.** A recipient added today can read what is written
//! after today; `oslo secret rotate` is the separate, deliberate step that gives them the rest, and
//! it is separate because it rewrites every file in the store and because "who could read this
//! before I changed it" is a question with a permanent answer.

use oslo::secrets::{Recipient, Store, conf};

use super::fail;

const USAGE: &str = "usage: oslo secret recipient [--export] | add [--from FILE] | rm RECIPIENT";

pub fn run(store: &Store, args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("list" | "ls") => {
            show(store);
            0
        }
        Some("--export") => {
            for recipient in &store.recipients {
                println!("{recipient}");
            }
            0
        }
        Some("add") => match wanted(&args[1..]) {
            Ok(recipients) => add(store, &recipients),
            Err(e) => fail(&e),
        },
        Some("rm" | "remove") => match wanted(&args[1..]) {
            Ok(recipients) => remove(store, &recipients),
            Err(e) => fail(&e),
        },
        Some(other) => {
            eprintln!("oslo secret recipient: {other}: no such subcommand");
            eprintln!("{USAGE}");
            2
        }
    }
}

/// The list, with what this binary makes of each.
pub fn show(store: &Store) {
    if store.recipients.is_empty() {
        println!("recipient (the key's own public half)   implied, until one is added");
        return;
    }
    for recipient in &store.recipients {
        match recipient.native() {
            Ok(_) => println!("recipient {recipient}"),
            Err(e) => println!("recipient {recipient}   UNUSABLE: {e}"),
        }
    }
}

/// The recipients named on the command line, or read out of the file `--from` names.
///
/// A file is one per line, `#` comments allowed, which is the shape `age` itself reads and the
/// shape `--export` writes.
fn wanted(args: &[String]) -> Result<Vec<Recipient>, String> {
    if let Some(rest) = args.first().and_then(|a| a.strip_prefix("--from")) {
        let path = match rest.strip_prefix('=') {
            Some(path) => path.to_string(),
            None => args.get(1).cloned().ok_or("--from needs a file")?,
        };
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
        return text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(Recipient::new)
            .collect();
    }
    if args.is_empty() {
        return Err("a recipient, or --from FILE".to_string());
    }
    args.iter().map(|arg| Recipient::new(arg)).collect()
}

/// **Checked before it is written.** A recipient this binary cannot encrypt to would fail on the
/// next `set` rather than now, which is the wrong end of the mistake to find out at.
fn add(store: &Store, recipients: &[Recipient]) -> i32 {
    for recipient in recipients {
        if let Err(e) = recipient.native() {
            return fail(&e);
        }
    }
    let mut declared = match conf::read() {
        Ok(conf) => conf,
        Err(e) => return fail(&e),
    };
    // The implied recipient is the key's own public half, and it stops being implied the moment a
    // list exists — so it is written down before anything joins it, or adding a colleague would
    // quietly stop encrypting to you.
    if store.recipients.is_empty() {
        match store.identities(false) {
            Ok(identities) => match identities.first() {
                Some(identity) => {
                    declared.add(&store.name, &format!("recipient {}", identity.to_public()));
                }
                None => return fail("this store has no key yet — `oslo secret key init` first"),
            },
            Err(e) => return fail(&e),
        }
    }
    for recipient in recipients {
        declared.add(&store.name, &format!("recipient {recipient}"));
    }
    match declared.write() {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

fn remove(store: &Store, recipients: &[Recipient]) -> i32 {
    let mut declared = match conf::read() {
        Ok(conf) => conf,
        Err(e) => return fail(&e),
    };
    let mut gone = 0;
    for recipient in recipients {
        let line = format!("recipient {recipient}");
        gone += declared.remove(&store.name, |written| written == line);
    }
    if gone == 0 {
        return fail(&format!("{}: none of those are its recipients", store.name));
    }
    match declared.write() {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}
