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

const USAGE: &str = "usage: oslo secret set|get|list|rm|where [NAME]\n\
                     \n\
                     \x20 set NAME    read a value from standard input and keep it encrypted\n\
                     \x20 get NAME    write that value to standard output\n\
                     \x20 list        the names kept here\n\
                     \x20 rm NAME     forget one\n\
                     \x20 where       the store and the key, and which of them may be committed";

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
        // **Two directories with opposite rules, so say which is which.** The whole point of the
        // store being encrypted is that it can be committed; the whole point of the key being
        // elsewhere is that it cannot.
        Some("where" | "paths") => {
            let unknown = std::path::PathBuf::from("(nowhere: no $HOME)");
            println!(
                "store     {}   encrypted, safe to commit",
                secrets::directory()
                    .unwrap_or_else(|| unknown.clone())
                    .display()
            );
            println!(
                "key       {}   never commit this",
                secrets::identity_path().unwrap_or(unknown).display()
            );
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
///
/// **At a terminal it is asked for instead**, masked, by the shell's own `ui input`. Standard input
/// there is the keyboard, so reading it to end of file means the value is typed in the clear, into
/// the scrollback, and finished with a Ctrl-D nobody is told about.
fn set(name: &str) -> i32 {
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
