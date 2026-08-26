//! Turn [argc](https://github.com/sigoden/argc)-annotated scripts into carapace completion specs.
//!
//! ```console
//! $ cargo run -p argc-to-spec -- <argc-completions>/completions share/completion
//! ```
//!
//! # It uses argc's own parser, not a reader of its own
//!
//! An argc declaration is a comment with a grammar — `# @option -f --file <FILE>`, `# @arg
//! path*[`_choice_path`]`, `# @cmd`, `# @alias` — and the grammar has corners: notation counts,
//! `*` against `+`, inherited flags, choices that are a list and choices that are a function.
//! Writing a second reader for it would be writing a second set of those corners, and the two
//! would disagree on exactly the scripts nobody tested. oslo already vendors argc; this asks it.
//!
//! # What a dynamic choice is, and why it does not survive
//!
//! Most of the value in a real argc completion is a `_choice_*` **bash function inside the
//! script** that lists what exists right now. A spec file is data and holds no functions, so those
//! are dropped and counted — the count is large, and the report says so rather than implying the
//! conversion was lossless.
//!
//! Calling back into the original script was tried and does not work: the scripts `source` an
//! 853-line bash helper library through a `$ROOT_DIR` they expect to be set, and end in an
//! `eval` of `argc --argc-eval` rather than a guard. Keeping those completions means installing
//! that whole bash environment, which is an integration rather than a conversion — see
//! `docs/features/completion-and-matching.md`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

mod emit;
mod map;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut source: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut only: Option<BTreeSet<String>> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--only" => {
                only = args
                    .next()
                    .map(|list| list.split(',').map(str::to_string).collect())
            }
            "-h" | "--help" => usage(0),
            _ if source.is_none() => source = Some(PathBuf::from(arg)),
            _ if out.is_none() => out = Some(PathBuf::from(arg)),
            other => {
                eprintln!("argc-to-spec: {other}: unexpected argument");
                usage(2);
            }
        }
    }
    let (Some(source), Some(out)) = (source, out) else {
        usage(2);
    };

    let mut report = map::Report::default();
    let mut written = 0usize;
    let mut empty = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();

    if let Err(problem) = fs::create_dir_all(&out) {
        eprintln!("argc-to-spec: {}: {problem}", out.display());
        std::process::exit(1);
    }

    for (command, path) in scripts(&source) {
        if only
            .as_ref()
            .is_some_and(|wanted| !wanted.contains(&command))
        {
            continue;
        }
        let source_text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(problem) => {
                failed.push((command, problem.to_string()));
                continue;
            }
        };
        // argc's own parser. A script it refuses is one oslo would refuse too, so the failure is
        // reported under the command's name rather than swallowed.
        let exported = match argc::export(&source_text, &command) {
            Ok(value) => value,
            Err(problem) => {
                failed.push((command, problem.to_string()));
                continue;
            }
        };
        let spec = map::command(&exported, &mut report);
        // **A spec with nothing in it is worse than no spec.** It is a file saying "this command
        // has no completions", where the reader would rather fall through to its own path
        // completion than read that.
        if spec.flags.is_empty()
            && spec.persistent.is_empty()
            && spec.commands.is_empty()
            && spec.positional.is_empty()
            && spec.positional_any.is_empty()
        {
            empty += 1;
            continue;
        }
        let text = emit::document(&spec, &command);
        if let Err(problem) = fs::write(out.join(format!("{command}.yaml")), text) {
            failed.push((command, problem.to_string()));
            continue;
        }
        written += 1;
    }

    println!("written  {written}");
    println!("failed   {}", failed.len());
    println!("empty    {empty}");
    for (command, why) in failed.iter().take(30) {
        println!("  {command}: {}", why.lines().next().unwrap_or(""));
    }
    if failed.len() > 30 {
        println!("  … and {} more", failed.len() - 30);
    }
    println!("\ncarried across:");
    println!("  flags            {}", report.flags);
    println!("  subcommands      {}", report.subcommands);
    println!("  static choices   {}", report.static_choices);
    println!("\nnot carried across:");
    println!("  dynamic choices  {}", report.dropped_choices);
    println!("  notations        {}", report.notations);
    println!("  env declarations {}", report.envs);
}

fn usage(code: i32) -> ! {
    eprintln!(
        "usage: argc-to-spec <scripts-dir> <out-dir> [--only a,b]

  <scripts-dir>          a directory of argc-annotated `.sh` files, one per command
  <out-dir>              where the `.yaml` specs are written
  --only a,b             convert only these commands"
    );
    std::process::exit(code)
}

/// Every `<command>.sh` in `dir`, sorted, so a run is reproducible.
fn scripts(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        eprintln!("argc-to-spec: {}: cannot read", dir.display());
        std::process::exit(1);
    };
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            let command = name.strip_suffix(".sh")?;
            // A leading underscore is argc-completions' own convention for a shared fragment
            // rather than a command anybody types.
            (!command.is_empty() && !command.starts_with('_'))
                .then(|| (command.to_string(), path.clone()))
        })
        .collect();
    found.sort();
    found
}
