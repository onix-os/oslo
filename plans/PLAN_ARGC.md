# argc in oslo: a shell that parses its own scripts' arguments

`argc` is a CLI framework for bash: you declare options as comments (`# @option -t --tries <NUM>`),
call `eval "$(argc --argc-eval "$0" "$@")"`, and the shell wakes up with `$argc_tries` set, help
text written, and errors reported. The proposal is to vendor it so that **oslo is that parser** —
`oslo --argc-eval …` for bash scripts, and something better than eval for oslo's own.

**Done**, on `feat/argc`. Built, measured, `make verify` green, and written up in
[`docs/features/argc.md`](docs/features/argc.md). What the analysis got right and wrong is at the
bottom.

## What it actually is, measured

| | |
|---|---|
| version | 1.24.0 |
| licence | **MIT OR Apache-2.0** — compatible with oslo's MIT, unlike `full_moon`'s MPL |
| size | 9,047 lines of Rust across 18 files |
| shape | a **library** with a thin binary on top; features are already split for embedding |
| upstream | active, and its `Cargo.toml` says in as many words: *"Feature required for argc the application. Should be disabled when depending on argc as a library."* |

The three files that matter are `parser.rs` (1,170 lines — the comment tags, in `nom`),
`matcher.rs` (1,626 — argv against the declaration) and `command/mod.rs` (1,084 — the model).

## The discovery that decides the design

`argc::eval()` does **not** return bash. It returns `Vec<ArgcValue>`:

```rust
pub enum ArgcValue {
    Single(String, String),          // argc_tries=3
    Multiple(String, Vec<String>),   // argc_files=( a b )
    PositionalSingle(String, String),
    Env(String, String),
    CommandFn(String),               // which function to call
    Error((String, i32)),            // what to print, and the status to exit with
    …
}
```

`ArgcValue::to_bash(&values)` is one *renderer* of that, behind its own `eval-bash` feature. So the
text protocol is not the interface — it is one of two possible front ends, and the second one is
oslo applying those values **straight to its own `Environment`**. No `eval`, no subprocess, no
quoting round trip, no bash.

That is the whole case for vendoring rather than shelling out to the `argc` binary.

## What it costs — measured, not guessed

A minimal binary that calls `argc::eval` and prints `to_bash`, built at oslo's own release profile
(`lto = "fat"`, `codegen-units = 1`, `strip`, `opt-level = "s"`):

```text
baseline (empty main)                          292,376 bytes
+ argc {eval, eval-bash, native-runtime}       731,600 bytes     → +439 KB
+ compgen on top of that                       732,000 bytes     → +400 bytes
```

**439 KB.** For scale, against oslo's own optional features: `direnv` is 200 KB, `plugin` 88 KB,
`scratch` smaller still. This would be the largest optional thing in the tree, and it is not close.

Dependencies it brings, against what oslo already links:

| already in oslo | new |
|---|---|
| `either`, `serde`, `serde_json`, `unicode-width` | `anyhow`, `convert_case`, `indexmap`, `nom` 8, `shell-words`, `natord` |

`nom` is the notable one: a second parser-combinator library in a tree whose entire argument is that
it owns its parsers. It parses *comments*, not a language, which is a defensible line — but it is a
line, and it should be drawn on purpose.

**Conclusion: a cargo feature, `argc`, off in `oslo-minimal`.** A `/bin/sh` does not need an argument
parser for other people's scripts.

## Three ways in, in ascending order of interest

### 1. `oslo --argc-eval` — being the argc binary

```sh
eval "$(oslo --argc-eval "$0" "$@")"      # in a #!/usr/bin/env bash script
```

Verified working from the library alone, no argc binary involved — this is real output from the
test build above, on argc's own `examples/demo.sh`:

```text
argc_force=1
argc_tries=3
argc_source=http://x.com
argc__args=( demo.sh download --force -t 3 http://x.com out.bin )
argc__fn=download
download http://x.com out.bin
```

The work is small: `src/cli.rs:190` is a plain long-option match, and `--argc-eval` joins
`--version`, `--login`, `--posix` there. It must swallow the rest of argv verbatim, the way `-c`
already does. `--argc-compgen`, `--argc-completions`, `--argc-build`, `--argc-export` are the same
shape and can follow one at a time.

**This makes oslo a drop-in for the `argc` binary**, which is the point: somebody's existing
`Argcfile.sh` and every script already written against argc keep working, with one fewer program
installed.

### 2. An `argc` builtin — for scripts oslo itself runs

```sh
#!/usr/bin/env oslo
# @option -t --tries <NUM>   how many times
argc "$@"                    #  ← in place of  eval "$(argc --argc-eval "$0" "$@")"
```

The builtin calls `argc::eval` and applies the `ArgcValue`s to the live `Environment` — `set_var`,
arrays, positional parameters, then calls the chosen function. What that buys over the eval line:

- **No subprocess and no eval.** One `fork`+`exec` and a quoting round trip per invocation, gone.
- **No bash.** The eval path needs a shell that speaks `name=( a b )`; the native path sets the
  variable directly.
- **Errors are oslo's.** `argc` reports a bad flag by printing and exiting; a builtin can report it
  the way every other builtin does, with the shell's own error machinery.
- **It works for a script with no file.** See the trap below, which is not hypothetical.

### 3. Completion — the part nobody else can do

`argc::compgen` already knows every flag, option, subcommand and choice a script declares. oslo has
its own completion engine and a provider registry (`crates/oslo-ui/src/completion/provider.rs`) that
was built for exactly this shape. Wiring one to the other means:

```text
deploy --<Tab>        → --force  --tries  --dir      from the script's own comments
```

for **every** argc-shaped script on `$PATH`, with no `_argc` shim, no `complete -F`, no generated
completion file to install and keep in step. argc ships completion scripts for seven shells to
achieve what oslo would get by calling a function.

And it extends to macros: a script stored in the macro database has no file for argc's runtime to
read — but oslo has the source in hand, so `oslo macros` scripts can carry `# @option` comments and
complete like anything else. That is a capability neither project has separately.

## The trap: `$0` and scripts with no file

A macro-stored script is executed from a `memfd`, and for a shell interpreter oslo repairs `$0` to
the script's *name* — verified: `greet` prints `greet`, not `/proc/self/fd/3`. So inside such a
script:

```sh
eval "$(oslo --argc-eval "$0" "$@")"     # $0 is `deploy` — not a path — and this fails
```

The bash idiom **cannot** work for a script that has no file, whichever binary evaluates it. The
builtin has no such problem: it never needs a path, because the shell already holds the source.
This is an argument for doing (2) rather than stopping at (1), and it should be stated in the docs
rather than discovered.

## The runtime seam

`argc::Runtime` is a 20-method trait — `env_var`, `which`, `read_dir`, `read_to_string`, `chdir`,
`current_exe`, and one that matters:

```rust
fn exec_bash_functions(&self, script_file: &str, functions: &[&str],
                       args: &[String], envs: HashMap<String, String>) -> Option<Vec<String>>;
```

argc calls this to run a script's *own* bash functions — a computed default
(`# @option --dir=$(pwd)`) or a choice function (`# @arg host[`_choice_host`]`). Upstream shells out
to bash to do it. **oslo runs them in its own interpreter**, which is faster, needs no bash on the
machine, and again works for a script with no file.

oslo already has every other method: `which` is the `$PATH` search, `env_var` is the `Environment`,
`read_dir` and the rest are `std::fs`. Implementing `Runtime` for oslo also drops argc's `which`
crate dependency.

## Where it lives, and one thing that has to change

`vendor/argc`, by the rule in `vendor/README.md`: *somebody else's code, kept close to upstream and
deliberately not restyled*. That file also says the two crates there are hard forks with **no
upstream to sync with** — which stops being true the moment argc lands, since it is actively
developed and worth following. The README needs a third row and an honest sentence about what
tracking upstream means. Modifications oslo makes (the `Runtime` impl belongs on oslo's side of the
line; a native `to_environment` renderer probably belongs upstream-shaped) should be a short, listed
set, so a rebase onto 1.25 is a readable diff rather than an archaeology exercise.

## What was built

All six steps, in two commits.

1. **`vendor/argc` at 1.24.0** — a workspace member with `src/bin/` removed, the `application`
   feature and four binary-only dependencies gone with it, `#![allow(…)]` at the top of `lib.rs` the
   way the other vendored crates carry theirs, and `pub use anyhow;` added so a caller can name
   `anyhow::Result` without depending on the crate. `vendor/README.md` now admits there is an
   upstream to track and lists those three modifications, so a rebase onto 1.25 is a readable diff.
2. **The `argc` cargo feature**, off by default, through `oslo-shell` and `oslo-runtime`; and
   `argc::Runtime` implemented over the shell — `$PATH` through the `hash` table, variables through
   the `Environment`, and `exec_bash_functions` running a script's own helpers in a command
   substitution rather than in a bash it has to find.
3. **`oslo --argc-eval`** — verified against a real bash script, which is the whole claim.
4. **The `argc` builtin**, applying `ArgcValue`s directly. Two bugs found by doing it: `args[0]` is
   the builtin's own name and had to be dropped, and `--help` has to raise the shell's `Exit` or the
   script runs its body with nothing set.
5. **Completion** through the provider registry, for `$PATH` scripts *and* stored macros. Verified
   at a live prompt: `ship --<Tab>` offered `--dry-run` and `--tries` for a script with no file.
6. **Docs**: `docs/features/argc.md`, the README, the feature index, and the `$0` trap said out loud.

Three files crossed the 600-line limit on the way and were split — `redact.rs` and `repl.rs` lost
their test modules to `#[path]` files, and `session.rs` lost `Bound`/`Step` to `session/keys.rs`.

## Measured, in the end

```text
every feature but argc     6,160,768 bytes
with argc                  6,476,160            +315,392   (308 KB)
the whole release build    6.25 MB
minimal, with none of it   5.17 MB
```

The analysis predicted 439 KB from an isolated test binary; the real cost is 308 KB, because fat LTO
shares what oslo already links. `vendor/argc` is built at `opt-level = 3` against the profile's `"s"`,
beside `oslo-ui` and `brush-parser`: the parse runs once per invocation of a script that uses it,
which is the shape that pays for speed.

**The `$0` trap was real.** Confirmed rather than assumed: a stored script's `$0` is its own name, so
the bash `eval "$(… "$0" …)"` idiom cannot work there and the builtin is the only door. It is in the
feature page under its own heading.

## What would make me not do it

- **439 KB** is a lot for a feature that only helps people who write argc-shaped scripts. If the
  answer is "this is for me and my scripts", it is worth it; if it is "the distribution ships this
  as `/bin/sh`", the feature flag is doing the work and the default should be off.
- **A second parser-combinator library.** Defensible for comments; worth saying no to if the answer
  is "we could parse `# @option` lines in 200 lines of our own code" — which is true, and which
  would also throw away compatibility with every argc script that already exists. That is the real
  trade, and it is a product decision rather than a technical one.
