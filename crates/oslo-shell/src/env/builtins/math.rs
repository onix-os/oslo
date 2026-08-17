//! `math` — a calculator at the prompt, with units.
//!
//! ```text
//! math 5 km in miles              3.10685596119 miles
//! math '9.8 m/s^2 * 70 kg'        686 kg·m·s⁻²
//! math 255 in hex                 0xff
//! math --unit '20 degC in degF'   68
//! ```
//!
//! # Why the arguments are joined rather than parsed
//!
//! `math 2 + 2` has to work, and the shell has already split that into three words. So every
//! operand is joined back with spaces and handed to the engine whole — which also means quoting is
//! only needed where the *shell* would otherwise act: `math '5 * 3'` because `*` globs, and
//! `math 5 km in miles` bare because none of those words mean anything to the shell.
//!
//! # What it prints
//!
//! One line: the answer, unit included. `--unit` and `--value` print one half each, for the case
//! where the answer is going into another command rather than onto a screen — `$(math --value '5
//! km in miles')` is a number a script can compare.
//!
//! # Which `-` is an option
//!
//! Only the `--long` ones, plus the short flags spelled out above. Everything else beginning with
//! a `-` is an operand, because `math -5` and `math -40 degC in degF` are the ordinary way to write
//! a negative number and there is no reading of them as a flag worth preferring. A mistyped option
//! still gets an error rather than a confusing arithmetic failure, since `--valu` is the shape a
//! mistake actually takes.
//!
//! # There are no variables here
//!
//! Every run builds a fresh scope and drops it, so `math 'x = 5'` is refused rather than answered:
//! it could only report `5` and forget the name, and the line after it — the one that wanted `x` —
//! would fail instead. Remembering is what `oslo.math.session()` is for, and the refusal says so.
//! This is the split on purpose: the builtin answers one question, Lua holds a conversation.

use crate::env::origin_now;
use crate::env::scope::Environment;
use oslo_base::error::Result;

pub fn builtin_math(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut wants = Show::All;
    let mut words: Vec<&str> = Vec::new();
    let mut only_operands = false;

    for arg in args.iter().skip(1) {
        if !only_operands && arg.len() > 1 {
            match arg.as_str() {
                "--" => only_operands = true,
                "--value" | "-v" => wants = Show::Value,
                "--unit" | "-u" => wants = Show::Unit,
                "--kind" | "-k" => wants = Show::Kind,
                "--units" => return Ok(list_units()),
                "--functions" => return Ok(list_functions()),
                "--help" | "-h" => {
                    println!("{HELP}");
                    return Ok(0);
                }
                // A `-` in front of a number is a minus sign, not a flag. Only the `--long` form
                // is worth an error, because that is the shape a mistyped option actually has:
                // `math -5` and `math -pi` are arithmetic, and demanding `math -- -5` for them
                // makes the escape hatch part of ordinary use.
                other if other.starts_with("--") => {
                    eprintln!("{}math: {other}: unknown option", origin_now());
                    eprintln!("{USAGE}");
                    return Ok(2);
                }
                _ => words.push(arg),
            }
            continue;
        }
        words.push(arg);
    }

    if words.is_empty() {
        eprintln!("{}math: there is nothing to work out", origin_now());
        eprintln!("{USAGE}");
        return Ok(2);
    }

    // Joined with a space, so `math 5 km in miles` is the same question as `math '5 km in miles'`.
    let source = words.join(" ");
    match oslo_math::calculate(&source) {
        Ok(answer) => {
            match wants {
                Show::All => println!("{}", answer.text),
                Show::Value => println!("{}", oslo_math::format::number_text(answer.number)),
                Show::Unit => println!("{}", answer.unit),
                Show::Kind => println!("{}", answer.dimension),
            }
            Ok(0)
        }
        Err(why) => {
            eprintln!("{}math: {why}", origin_now());
            Ok(1)
        }
    }
}

/// Which part of the answer to print.
enum Show {
    All,
    Value,
    Unit,
    Kind,
}

fn list_units() -> i32 {
    let mut names: Vec<&str> = oslo_math::units::UNITS.iter().map(|u| u.name).collect();
    names.sort_unstable();
    names.dedup();
    println!("{}", names.join(" "));
    println!();
    println!(
        "Any of the SI prefixes may go in front of the ones that take them: k, M, G, m, µ, n."
    );
    0
}

fn list_functions() -> i32 {
    for (name, about) in oslo_math::functions::NAMES {
        println!("{name:<10}{about}");
    }
    0
}

const USAGE: &str = "math: usage: math [--value|--unit|--kind] EXPRESSION...";

const HELP: &str = "\
USAGE
  math EXPRESSION...

  Work out an expression, with units.

ARGUMENTS
  -v, --value         print the number alone, for a script to read
  -u, --unit          print the unit alone
  -k, --kind          print what kind of thing it is: length, a number, length·time⁻¹
      --units         every unit it knows
      --functions     every function it knows

EXAMPLES
  math 2 + 2
  math 5 km in miles
  math '9.8 m/s^2 * 70 kg'
  math 255 in hex
  math '0xff | 0x0f'
  math '20% of 250'
  math '1 GiB in MB'
  math -40 degC in degF     a leading minus is a minus, not an option

NOTES
  Nothing is remembered between runs, so there are no variables here. A
  session in Lua keeps them:  s = oslo.math.session(); s:eval('r = 3')";

#[cfg(test)]
#[path = "math/tests.rs"]
mod tests;
