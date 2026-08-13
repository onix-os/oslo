//! `oslo secret` — keep a value encrypted, hand it out when something asks.
//!
//! ```sh
//! oslo secret set deploy-token          # reads the value from stdin, or asks for it
//! oslo secret get deploy-token          # writes it to stdout, and nowhere else
//! oslo secret list
//! oslo secret rm deploy-token
//! ```
//!
//! The store and the reasoning are in [`oslo::secrets`]; this is the command line onto it.

use oslo::secrets;

const USAGE: &str = "usage: oslo secret set|get|list|rm [NAME]\n\
                     \n\
                     \x20 set NAME    read a value from standard input and keep it encrypted\n\
                     \x20 get NAME    write that value to standard output\n\
                     \x20 list        the names kept here\n\
                     \x20 rm NAME     forget one";

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("set") => match args.get(1) {
            Some(name) => set(name),
            None => usage(),
        },
        Some("get") => match args.get(1) {
            Some(name) => get(name),
            None => usage(),
        },
        Some("rm" | "remove" | "forget") => match args.get(1) {
            Some(name) => match secrets::forget(name) {
                Ok(()) => 0,
                Err(e) => fail(&e),
            },
            None => usage(),
        },
        Some("list" | "ls") | None => {
            for name in secrets::names() {
                println!("{name}");
            }
            0
        }
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

/// Read the value from standard input and keep it.
///
/// **A trailing newline is dropped**, because the value came from a line somebody typed or from a
/// `printf` in a script, and a token with `\n` on the end fails authentication in a way that takes
/// an hour to find.
fn set(name: &str) -> i32 {
    use std::io::Read;
    let mut value = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut value) {
        return fail(&format!("cannot read standard input: {e}"));
    }
    if value.last() == Some(&b'\n') {
        value.pop();
    }
    match secrets::set(name, &value) {
        Ok(()) => 0,
        Err(e) => fail(&e),
    }
}

/// Write the value to standard output, with nothing added to it.
fn get(name: &str) -> i32 {
    use std::io::Write;
    match secrets::get(name) {
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

fn fail(message: &str) -> i32 {
    eprintln!("oslo secret: {message}");
    1
}

fn usage() -> i32 {
    eprintln!("{USAGE}");
    2
}
