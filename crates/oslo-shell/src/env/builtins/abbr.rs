//! `abbr` — define a word that expands as you type it.
//!
//! ```text
//! abbr gco 'git checkout'          define one
//! abbr --anywhere brc '~/.bashrc'  let it fire outside command position
//! abbr                             list them
//! abbr -e gco                      remove one
//! ```
//!
//! See [`oslo_ui::abbr`] for why this exists beside `alias` rather than instead of it:
//! an alias changes what a command means, an abbreviation only saves typing.

use super::variables::quoting::single_quoted;
use crate::env::Environment;
use oslo_base::error::Result;
use oslo_ui::abbr::{self, Placement};

pub fn builtin_abbr(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let operands = &args[1..];
    if operands.first().is_some_and(|a| a == "--help" || a == "-h") {
        println!("Usage: abbr [--anywhere] NAME EXPANSION");
        println!("       abbr -e NAME            remove one");
        println!("       abbr                    list them");
        println!();
        println!("A `%` in EXPANSION says where to leave the cursor.");
        return Ok(0);
    }

    if operands.is_empty() {
        for (name, entry) in abbr::all() {
            let where_ = match entry.placement {
                Placement::Anywhere => "--anywhere ",
                Placement::Command => "",
            };
            println!("abbr {where_}{name} {}", single_quoted(&entry.expansion));
        }
        return Ok(0);
    }

    if operands[0] == "-e" || operands[0] == "--erase" {
        let Some(name) = operands.get(1) else {
            eprintln!("abbr: -e: a name is required");
            return Ok(2);
        };
        if !abbr::remove(name) {
            eprintln!("abbr: {name}: not an abbreviation");
            return Ok(1);
        }
        return Ok(0);
    }

    let mut placement = Placement::Command;
    let mut rest = operands;
    if rest[0] == "--anywhere" {
        placement = Placement::Anywhere;
        rest = &rest[1..];
    }

    let (Some(name), Some(expansion)) = (rest.first(), rest.get(1)) else {
        eprintln!("abbr: usage: abbr [--anywhere] NAME EXPANSION");
        return Ok(2);
    };
    // The rest of the words join with spaces, so `abbr gc git commit` works without quoting —
    // which is what someone reaches for first, and refusing it teaches nothing.
    let expansion = if rest.len() > 2 {
        rest[1..].join(" ")
    } else {
        expansion.to_string()
    };
    abbr::add(name, &expansion, placement);
    Ok(0)
}
