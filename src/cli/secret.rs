//! `oslo secret` — keep a value encrypted, hand it out when something asks.
//!
//! ```sh
//! oslo secret set deploy-token          # asks for the value at a terminal, or reads stdin
//! oslo secret get deploy-token          # writes it to stdout, and nowhere else
//! oslo secret run TOKEN=deploy-token -- curl …
//! oslo secret list
//! oslo secret rm deploy-token
//! ```
//!
//! Every one of them takes `--store NAME`, which is also `$OSLO_SECRET_STORE`. The store, its keys
//! and its recipients are in [`oslo::secrets`]; `key` and `recipient` below are the commands that
//! write that configuration, and `where` is the one that shows it.

use oslo::secrets::{self, Store};

mod key;
mod recipient;
mod run;

const USAGE: &str = "usage: oslo secret [--store NAME] COMMAND\n\
                     \n\
                     \x20 set NAME      read a value from standard input and keep it encrypted\n\
                     \x20 get NAME      write that value to standard output\n\
                     \x20 run VAR=NAME -- CMD…   run CMD with VAR set to it, and nothing else\n\
                     \x20 list          the names kept here\n\
                     \x20 rm NAME       forget one\n\
                     \x20 rotate        re-encrypt everything to the current recipients\n\
                     \x20 key           where this store's keys come from\n\
                     \x20 recipient     who this store encrypts to\n\
                     \x20 stores        every store this machine knows about\n\
                     \x20 where         the store and the key, and which of them may be committed";

pub fn run(args: &[String]) -> i32 {
    // Said once, wherever the command is going: a key under a `.git` is one `git add -A` from being
    // published, and the person it happens to did not choose it — they moved a directory.
    if let Some(repository) = secrets::identity_in_a_repository() {
        eprintln!(
            "oslo secret: the key is inside the git repository at {}",
            repository.display()
        );
        eprintln!(
            "oslo secret: move it with $OSLO_SECRET_IDENTITY, or the next commit publishes it"
        );
    }

    let (asked, args) = match store_argument(args) {
        Ok(taken) => taken,
        Err(e) => return fail(&e),
    };
    let store = || match Store::selected(asked.as_deref()) {
        Ok(store) => Ok(store),
        Err(e) => {
            eprintln!("oslo secret: {e}");
            Err(1)
        }
    };

    match args.first().map(String::as_str) {
        Some("set") => with_name(&args, |name| match store() {
            Ok(store) => set(&store, name),
            Err(code) => code,
        }),
        Some("get") => with_name(&args, |name| match store() {
            Ok(store) => get(&store, name),
            Err(code) => code,
        }),
        Some("rm" | "remove" | "forget") => with_name(&args, |name| match store() {
            Ok(store) => match store.forget(name) {
                Ok(()) => 0,
                Err(e) => fail(&e),
            },
            Err(code) => code,
        }),
        Some("list" | "ls") | None => match store() {
            Ok(store) => {
                for name in store.names() {
                    println!("{name}");
                }
                0
            }
            Err(code) => code,
        },
        Some("run") => match store() {
            Ok(store) => run::run(&store, &args[1..]),
            Err(code) => code,
        },
        Some("rotate") => match store() {
            Ok(store) => match store
                .encrypt_to()
                .and_then(|to| Ok((store.rotate()?, to.len())))
            {
                Ok((moved, to)) => {
                    println!("{moved} re-encrypted to {to} recipients");
                    0
                }
                Err(e) => fail(&e),
            },
            Err(code) => code,
        },
        Some("key") => match store() {
            Ok(store) => key::run(&store, &args[1..]),
            Err(code) => code,
        },
        Some("recipient" | "recipients") => match store() {
            Ok(store) => recipient::run(&store, &args[1..]),
            Err(code) => code,
        },
        Some("stores") => stores(),
        // **Two directories with opposite rules, so say which is which.** The whole point of the
        // store being encrypted is that it can be committed; the whole point of the key being
        // elsewhere is that it cannot.
        Some("where" | "paths") => match store() {
            Ok(store) => {
                let unknown = std::path::PathBuf::from("(nowhere: no $HOME)");
                println!(
                    "store     {}   encrypted, safe to commit",
                    store.directory.display()
                );
                println!(
                    "key       {}   never commit this",
                    secrets::identity_path()
                        .unwrap_or(unknown.clone())
                        .display()
                );
                println!(
                    "config    {}   names the keys, so never commit this either",
                    secrets::conf::path().unwrap_or(unknown).display()
                );
                println!();
                key::show(&store);
                recipient::show(&store);
                0
            }
            Err(code) => code,
        },
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            0
        }
        Some(other) => {
            eprintln!("oslo secret: {other}: no such subcommand");
            usage()
        }
    }
}

/// Take `--store NAME` out of the arguments wherever it is, so it can be written before or after
/// the command the way a person expects a global option to be.
fn store_argument(args: &[String]) -> Result<(Option<String>, Vec<String>), String> {
    let mut name = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.strip_prefix("--store") {
            Some("") => match args.next() {
                Some(value) => name = Some(value.clone()),
                None => return Err("--store needs the name of a store".to_string()),
            },
            Some(attached) => match attached.strip_prefix('=') {
                Some(value) => name = Some(value.to_string()),
                None => rest.push(arg.clone()),
            },
            None => rest.push(arg.clone()),
        }
    }
    Ok((name, rest))
}

fn with_name(args: &[String], then: impl FnOnce(&str) -> i32) -> i32 {
    match args.get(1) {
        Some(name) => then(name),
        None => usage(),
    }
}

/// Read the value from standard input and keep it.
///
/// **A trailing newline is dropped**, because the value came from a line somebody typed or from a
/// `printf` in a script, and a token with `\n` on the end fails authentication in a way that takes
/// an hour to find.
///
/// **At a terminal it is asked for instead**, masked, by the shell's own `ui input`. Standard input
/// there is the keyboard, so reading it to end of file means the value is typed in the clear, into
/// the scrollback, and finished with a Ctrl-D nobody is told about.
fn set(store: &Store, name: &str) -> i32 {
    use std::io::{IsTerminal, Read};
    let mut value = Vec::new();
    if std::io::stdin().is_terminal() {
        let asked = oslo_ui::ask::input(&oslo_ui::ask::Input {
            prompt: format!("{name}: "),
            password: true,
            required: true,
            ..Default::default()
        });
        match asked {
            oslo_ui::ask::Answer::Given(typed) => value = typed.into_bytes(),
            oslo_ui::ask::Answer::Cancelled => return 1,
            oslo_ui::ask::Answer::NoTerminal => return fail("nothing to read the value from"),
        }
    } else if let Err(e) = std::io::stdin().read_to_end(&mut value) {
        return fail(&format!("cannot read standard input: {e}"));
    }
    if value.last() == Some(&b'\n') {
        value.pop();
    }
    match store.set(name, &value) {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

/// Write the value to standard output, with nothing added to it.
fn get(store: &Store, name: &str) -> i32 {
    use std::io::Write;
    match store.get(name) {
        Ok(value) => {
            let mut out = std::io::stdout();
            if out.write_all(&value).is_err() || out.flush().is_err() {
                return fail("cannot write the value");
            }
            0
        }
        Err(e) => fail(&e),
    }
}

/// Every store declared, and the one a bare `oslo secret` means.
fn stores() -> i32 {
    let declared = match secrets::conf::read() {
        Ok(conf) => conf,
        Err(e) => return fail(&e),
    };
    let mut names: Vec<String> = declared.sections.iter().map(|s| s.name.clone()).collect();
    if !names.iter().any(|name| name == secrets::USER) {
        names.insert(0, secrets::USER.to_string());
    }
    let default = declared
        .default
        .unwrap_or_else(|| secrets::USER.to_string());
    for name in names {
        let here = if name == default { "*" } else { " " };
        match Store::named(&name) {
            Ok(store) => println!("{here} {name}\t{}", store.directory.display()),
            Err(e) => println!("{here} {name}\t{e}"),
        }
    }
    0
}

pub fn fail(message: &str) -> i32 {
    eprintln!("oslo secret: {message}");
    1
}

fn usage() -> i32 {
    eprintln!("{USAGE}");
    2
}
