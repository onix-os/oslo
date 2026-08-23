//! `oslo lua-api` — hand another program the client library, as source.
//!
//! ```lua
//! local src  = io.popen("oslo lua-api"):read("a")
//! local oslo = load(src)(my_transport)
//! local sh   = oslo.connect()
//! print(sh.env.get("PATH"))
//! ```
//!
//! # Why a command rather than a file path
//!
//! A path only works if the reader knows where oslo was installed, which differs between a distro
//! package, a nix profile and a `cargo install`. Everything already knows how to *run* `oslo` — it
//! is on `$PATH` by definition, since that is how it was reached. So the binary hands out its own
//! client, versioned with itself: there is no way to load a stub from one oslo and talk to another.
//!
//! It also means a sibling embeds nothing. hexe does not vendor a copy of `oslo.lua` that goes
//! stale; it asks the oslo it is actually talking to.
//!
//! # `--path` and `--verbs`, for the two questions that follow
//!
//! Where the socket would be, and what a peer may call. Both are answerable without connecting, and
//! both are the first thing anybody debugging this asks.

use oslo_runtime::lua::api::live;

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None => {
            print!("{}", live::CLIENT);
            0
        }
        Some("--path") => {
            println!(
                "{}",
                oslo_base::wire::socket_path("oslo", args.get(1).map(String::as_str)).display()
            );
            0
        }
        Some("--verbs") => {
            for (name, about) in live::VERBS {
                println!("{name:<14} {about}");
            }
            0
        }
        Some("--help" | "-h") => {
            println!("{USAGE}");
            0
        }
        Some(other) => {
            eprintln!("oslo lua-api: {other}: not an option\n\n{USAGE}");
            2
        }
    }
}

const USAGE: &str = "usage: oslo lua-api [--path [SESSION] | --verbs]\n\
     \n\
     With no option, prints the Lua client library on stdout. Load it in any Lua VM:\n\
     \n\
       local src  = io.popen(\"oslo lua-api\"):read(\"a\")\n\
       local oslo = load(src)(transport)\n\
       local sh   = oslo.connect()\n\
     \n\
     `transport` supplies one function, `connect(path, timeout_ms)`, answering a handle with\n\
     `send`, `recv` and `close`. Inside oslo that is `oslo.stream` and it is found automatically.\n\
     \n\
     OPTIONS\n  \
       --path [SESSION]  where the socket is, or would be\n  \
       --verbs           what a connected peer may call\n\
     \n\
     A shell serves nothing until it is asked to — see `oslo.live.serve()`.";
