# oslo

**O**nix **S**hell in **L**ua (**O**) — a POSIX-compatible shell in Rust, with Lua scripting
and fish-style interactive features. Linux only.

```sh
oslo                       # interactive REPL when stdin is a terminal, else read stdin
oslo script.sh arg1 arg2   # run a shell script
oslo build.lua             # run a Lua script — same command, no flag
oslo -c 'echo hello'       # run a shell command
oslo -s < script.sh        # read the program from standard input
oslo --help                # -c -s -i -l -e -x --lua --sh --version --help --
```

## Status

Beta. The core language works — pipelines, redirections, control flow, functions, expansion —
but see [Known gaps](#known-gaps) before relying on it as a login shell.

## Language support

| | |
|---|---|
| **Pipelines** | `a \| b \| c`, `!` negation, `&&` / `\|\|` |
| **Redirection** | `>` `>>` `<` `<>` `>\|` `2>` `2>&1` `&>` `&>>`, heredocs (`<<`, `<<-`), here-strings (`<<<`) |
| **Control flow** | `if`/`elif`/`else`, `while`, `until`, `for`, `case` (with glob patterns), `{ }`, `( )` |
| **Tests** | `test` / `[ ]`, and `[[ ]]` with glob matching (`[[ abc == a* ]]`) |
| **Functions** | `name() { ... }`, `local`, `return`, positional parameters |
| **Loop control** | `break [n]`, `continue [n]` |
| **Expansion** | `$var`, `${var}`, `${var:-d}` `${var:=d}` `${var:+a}` `${var:?e}` `${#var}` `${var%p}` `${var#p}`, `$(cmd)`, backticks, `$((expr))`, `~`, globs, IFS field splitting |
| **Jobs** | `cmd &` runs in the background; `$!` holds its pid |

Builtins, by what they act on:

| | |
|---|---|
| **Shell state** | `set` `shopt` `export` `readonly` `local` `declare` `typeset` `unset` `shift` `alias` `unalias` `hash` |
| **Control** | `exit` `break` `continue` `return` `eval` `exec` `source` `.` `command` `builtin` `caller` `:` `true` `false` |
| **Input and words** | `echo` `read` `mapfile` `readarray` `getopts` `let` |
| **Directories** | `cd` `pwd` `pushd` `popd` `dirs` |
| **Tests** | `test` `[` `[[` |
| **Jobs and signals** | `jobs` `fg` `bg` `disown` `wait` `kill` `trap` `suspend` |
| **Process** | `type` `umask` `times` `ulimit` |

## Lua

Lua is a first-class way to write a program for this shell, not an add-on: `oslo build.lua` and
`oslo deploy.sh` are the same command, and neither needs a flag saying which language it got.

The language is worked out from the strongest evidence available, in order:

1. **`--lua` or `--sh`**, if you passed one. Nothing else gets a vote.
2. **The shebang** — `#!/usr/bin/env lua` against `#!/bin/sh`. Matched on the interpreter's
   basename, so `lua5.4` counts and `/opt/lua/bin/bash` does not.
3. **The extension** — `.lua` against `.sh`/`.bash`/`.zsh`/`.ksh`.
4. **The text**, and only when it is unambiguous: constructs that would be a syntax error in the
   other language, such as `ipairs(` or `--[[` against `$(` or `esac`.

If even the text is ambiguous the answer is shell — that is the case where a file was handed to a
shell with nothing indicating otherwise, and guessing the other way would silently reinterpret
POSIX scripts. Give a file a shebang and the question never arises.

`-c` is always shell, whatever the text looks like: every `sh -c` idiom depends on that. Use
`oslo --lua -c 'print(1)'` for the other reading.

### Writing a program in Lua

A Lua program gets `arg` and `...` the way any Lua interpreter provides them, captures a command's
answer, and chooses the shell's exit status:

```lua
#!/usr/bin/env oslo
-- deploy.lua — run it as: oslo deploy.lua staging
local target = arg[1] or "dev"

local branch = oslo.capture("git rev-parse --abbrev-ref HEAD")
if branch.status ~= 0 then
  io.stderr:write("not a git checkout\n")
  oslo.exit(1)
end

for _, cfg in ipairs(oslo.glob("conf/*.conf")) do
  print(("%s -> %s"):format(cfg, target))
end

oslo.cd("/srv/" .. target)
oslo.set_var("DEPLOY_ENV", target)          -- exported, so children see it
oslo.exit(oslo.exec("./install.sh"))
```

The API, by what it acts on:

| | |
|---|---|
| **Commands** | `oslo.exec(cmd)` → status, output goes to the shell's stdout · `oslo.capture(cmd)` → `{out, status}` |
| **Variables** | `oslo.get_var(n)` · `oslo.set_var(n, v)` · `oslo.unset(n)` · `oslo.env()` → table of exported names |
| **Filesystem** | `oslo.get_pwd()` · `oslo.cd(path)` → `true` or `nil, err` · `oslo.glob(pat)` → array |
| **Shell** | `oslo.set_alias(n, t)` · `oslo.get_alias(n)` · `oslo.register_builtin(n, fn)` · `oslo.set_prompt(fn)` · `oslo.exit(code)` |
| **Arguments** | `arg[0]` the script, `arg[1..n]` its operands, `arg[-1]` the interpreter; the same list as `...` |

`oslo.capture` strips trailing newlines, as `$(cmd)` does. It has **no `err` field**: it captures
stdout and leaves stderr on the shell's own, so an `err` could only ever be empty — and a field
that is always empty reads as "no diagnostics" rather than "nobody looked". Fold them the way a
shell does when you want both: `oslo.capture("cmd 2>&1")`.

Failures answer the way Lua's own library does — `nil, message` rather than a raised error — so
`local ok, err = oslo.cd(p)` reads like `io.open`. An error is raised only for a mistake in the
calling script, such as an empty builtin name.

### Configuration

`~/.config/oslo/init.lua` is loaded at startup.

## Interactive

Syntax highlighting (valid commands green, unknown red), history hints, a completion dropdown
with descriptions, and a git-aware prompt. History is kept in `~/.oslo_history`.

## Building

```sh
make build     # build the shell
make run       # build and run the REPL
make test      # run the test suite
make verify    # the full gate: fmt, loc, check, test, clippy, rustdoc
make install   # install to $PREFIX/bin (default ~/.local/bin)
```

The minimum supported Rust version is 1.88 (edition 2024 plus let-chains); CI checks it on every
push, alongside the verify gate. oslo targets Linux and only Linux — it uses /proc, memfd and
Linux signal numbers directly rather than routing around them.

## Installing

```sh
make install                 # builds --release and installs to ~/.local/bin/oslo
make install PREFIX=/usr/local   # or anywhere else
make uninstall               # removes it again (same PREFIX)
```

`PREFIX` defaults to `$HOME/.local`, so the binary lands in `$HOME/.local/bin` — make sure that
directory is on your `PATH`. `DESTDIR` is honoured for staged/packaging installs.

Released binaries are statically linked against musl (`x86_64-unknown-linux-musl` and
`aarch64-unknown-linux-musl`), so they depend on no system libc and run on any Linux of the same
architecture — which matters for a login shell, since a shell that cannot start because its libc
moved is a shell that locks you out. `make install` builds against the host's libc instead; to
build a static one yourself:

```sh
rustup target add x86_64-unknown-linux-musl
sudo apt-get install -y musl-tools          # compiles mlua's vendored Lua for musl
CC_x86_64_unknown_linux_musl=musl-gcc \
RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --target x86_64-unknown-linux-musl --bin oslo
```

Do not also set the target's *linker* to `musl-gcc`. It builds and looks right, and produces a
binary that records a dynamic loader path from the build host and segfaults anywhere else; leaving
the linker alone lets rustc use the musl it ships. `readelf -l oslo | grep interpreter` should
print nothing.

Check it works before going further:

```sh
~/.local/bin/oslo -c 'echo ok'
```

### Using oslo as your login shell

Read [Known gaps](#known-gaps) first. A login shell that cannot parse your startup files locks
you out of your own account, and the recovery path (another terminal, or single-user mode) is
not always available on a remote machine.

A shell may only be a login shell if it is listed in `/etc/shells`:

```sh
sudo make install PREFIX=/usr/local            # system-wide, so it survives $HOME being unmounted
echo /usr/local/bin/oslo | sudo tee -a /etc/shells
chsh -s /usr/local/bin/oslo
```

`chsh` refuses any path not in `/etc/shells`, which is the check that stops you from setting a
shell that does not exist. Log out and back in for the change to take effect.

Before you commit to it, run oslo as a non-login shell for a while — start it from your current
shell, or set it as your terminal emulator's command — so that a breakage costs you a window
instead of a session.

To go back:

```sh
chsh -s /bin/bash
```

If you are already locked out, most systems let you log in over SSH with an explicit shell
(`ssh host -t /bin/bash -l`), or you can switch the entry back with `sudo chsh -s /bin/bash $USER`
from a rescue session.

## Testing as a distro's /bin/sh

```sh
scripts/alpine-vm.sh            # boot an Alpine VM with oslo as PID 1 and /bin/sh, run the suite
scripts/alpine-vm.sh --shell    # boot to an interactive oslo prompt in there instead
```

Needs `qemu-system-x86_64`, `cpio` and network access on the first run (the Alpine minirootfs and
kernel are cached afterwards). Alpine because it is musl and busybox: the static release binary has
to run where no glibc exists, and every utility in the image is a different implementation from the
ones the differential corpus compares against. See `PLAN-DISTRO.md` for what it found.

## File length

**No source file may exceed 600 lines.** Enforced by `scripts/check-loc.sh`, which runs as part
of `make verify`; check it directly with `make check-loc`.

A file that outgrows the limit has almost always taken on a second responsibility, and the point
where it breaches is usually the seam to split along. Split modules are named for **what they
contain** — `redirects.rs`, `quoting.rs`, `conditionals.rs` — never for their position
(`part1.rs`, `helpers2.rs`), which tells a reader nothing about where to look.

## Testing

`cargo test` runs four kinds of suite, deliberately different in what they can see:

- **In-process** — `tests/posix_shell_tests.rs` builds an AST, evaluates it and inspects
  `Environment` directly. Fast, but blind to anything that only goes wrong in a real process.
- **End-to-end** — most of `tests/`: `tests/common/mod.rs` spawns the real binary with stdin from
  `/dev/null` under a timeout, and the test asserts on stdout, stderr and exit status. This exists
  because the first kind cannot see a whole class of defect: a redirection dropped during AST
  conversion, or an exit status never propagated to `main`, leaves the environment looking
  perfectly correct while the shell is visibly broken.
- **Differential** — `tests/differential_tests.rs` runs every script in `tests/corpus/` through
  both oslo and bash and compares stdout and exit status (stderr by shape only, since diagnostic
  wording should differ). Each script names its oracle on the first line, `# mode: posix` or
  `# mode: bash`, and **both** shells are run in that mode; giving only bash the `--posix` was a
  harness bug that judged 304 cases against a mode oslo was not in. It is a ratchet:
  `tests/differential/expected_fail.rs` names each case oslo still gets wrong along with the
  finding that explains it, and the suite fails both when an unlisted case diverges *and* when a
  listed case starts passing. Closing a bug means deleting a line there.

  Bash is a moving specification, so a case may add `# needs-bash: 5.3` under its mode line. Four
  behaviours changed between 5.2 and 5.3 — whether a failing special builtin is fatal, `cd`'s
  status for too many operands, and two column widths — and oslo follows 5.3. Against an older
  oracle those cases are skipped and counted rather than blamed on oslo, and the suite prints the
  oracle's version and the skip list on every run so a CI image that ages cannot quietly stop
  testing them. The oracle must be bash 4 or newer.
- **Fuzz replay** — `cargo test --manifest-path fuzz/Cargo.toml --lib` runs `tests/corpus/` and
  `fuzz/seeds/` through the three `cargo-fuzz` targets on stable, with no nightly and no
  libFuzzer. `fuzz/known/` is the second ratchet, in the same two directions: an input that still
  crashes or hangs the shell lives there with a note, and the suite fails the day it stops
  reproducing. It is empty today. `fuzz/README.md` covers running the fuzzer proper.

The line editor is covered without a pty: `OsloHelper` is public, so `tests/interactive_tests.rs`
and `src/interactive/tests.rs` call `complete`, `hint`, `highlight` and `input_status` directly
against temporary directories.

One rule holds across all of it: **a test that writes `environ`, the working directory or the
umask spawns the binary rather than running in process.** libtest runs `#[test]` functions as
threads of one process, and those three belong to the process; `Environment::set_var` on an
exported name reaches `unsafe { std::env::set_var }`, which racing another thread's `env::vars()`
walk is undefined behaviour rather than flakiness. `tests/posix_shell_tests.rs` and
`tests/subshell_state_tests.rs` split into `spawned` and `in_process` modules for exactly this.

## Architecture

```
main.rs          script, -c, stdin and REPL entry points
cli.rs           argv parsing for the binary
lexer/           hand-written POSIX scanner (word re-lexing for the adapter)
parser/
  brush_adapter  brush-parser AST -> oslo AST  (the only parsing path)
ast/             the shared AST
expand/          parameter expansion, globbing, IFS splitting, arithmetic
exec/            fork/exec, pipelines, redirections, job control
env/             variables, scopes, aliases, functions, builtins
lua/             mlua bindings
interactive/     rustyline helper: completion, highlighting, dropdown, prompt
```

Parsing goes through [`brush-parser`](https://crates.io/crates/brush-parser), a spec-compliant
bash parser, and `brush_adapter` translates its AST into oslo's simpler one. That is the only
path: input brush or the adapter rejects becomes a syntax error with a position, and the shell
exits 2. There used to be a second, hand-written parser used as a fallback, which had no
here-document support and therefore executed heredoc *bodies* as commands; it is gone. The
hand-written lexer stays — the adapter re-lexes brush's raw word text through it.

brush-parser is used **unmodified**, straight from crates.io. It was briefly vendored to carry one
grammar patch — the tokenizer takes the longest match, so an arithmetic `for` loop with an empty
condition has its two section separators fused into the single `;;` that ends a `case` item, and
`for ((;;))` fails to parse where `for (( ; ; ))` succeeds. The fix is a 28-line alternative in one
grammar rule, but carrying it meant keeping 10,181 lines of someone else's parser across 247 files
in this repo, to own a change smaller than this paragraph. That is the wrong side of the trade, so
the fork is gone and the construct is a [known gap](#known-gaps). The patch belongs upstream.

## Known gaps

Each of these is reproducible against the binary, and all but the last are differences from bash.
The first four have a corpus case, named in `tests/differential/expected_fail.rs`, so the
differential suite watches them and will fail the day one is fixed and the line is not deleted.
The rest are recorded only here: a corpus case for a known-missing feature buys a second copy of
this list and a row in the ratchet, which is worth it for a bug and not for an absence.

- `for ((;;))` is a syntax error when the section separators touch. Write `for (( ; ; ))` instead;
  the ordinary `for ((i=0;i<3;i++))` is unaffected. The cause is upstream: brush's tokenizer takes
  the longest match and fuses the two `;` into the single `;;` that ends a `case` item, so the
  arithmetic-for rule never sees them. A small alternative in that one grammar rule fixes it, but
  only by vendoring the whole parser — better sent upstream than carried here.
- Process substitution (`<(cmd)`, `>(cmd)`) is refused by name with exit status 2. Refusing is
  deliberate — silently dropping the argument made `diff <(a) <(b)` report false success — but the
  `/dev/fd/N` implementation is not written.
- `coproc` and `select` are likewise refused by name. Both need machinery (job control, and a
  prompt plus `REPLY`) out of proportion to how often scripts use them.
- A failing *special* builtin does not exit a POSIX-mode shell. `oslo --posix -c 'export
  BAD-NAME=1; echo alive'` prints `alive`; `bash --posix` stops at the `export`. The narrower
  POSIX rule that a failed *variable assignment* is fatal does hold — `readonly r=1; r=2` ends the
  shell under `--posix` and does not outside it, as in bash.
- Arrays are indexed only. `declare -A` reports that associative arrays are unsupported rather
  than pretending. Slicing (`${a[@]:1}`, `${@:2}`) and element-wise operators (`${a[@]#pat}`,
  `${@^^}`) do work on a whole array and on the positional parameters; the list-valued
  `${a[@]:-default}` family is still rejected instead of evaluated.
- `shopt` exists, but `autocd` is the only option it can switch. Every other option it knows
  reports the state oslo actually has and *fails* when asked for the other one, so
  `shopt -s globstar` is an error rather than a lie: `**` still behaves as `*`.
- The command hash table only holds what an explicit `hash name` put there. `hash -r`, the
  reporting and the completion cache are all wired; the `PATH` search itself does not record
  what it resolved yet.
- No `SECONDS`, `RANDOM`, `LINENO`, `/dev/tcp`, restricted mode, or `vi`/`emacs` editing modes.
- History expansion (`!!`, `!$`, `^old^new`) applies to REPL lines only, not to `-c` or scripts —
  the same rule bash uses.

### Input limits

Two ways to hang the shell with *data* survived until Round 11, and both were found by the
fuzzer rather than by the corpus:

- A no-break space — or any other character in Unicode `White_Space` that is not a shell blank —
  anywhere in a command made the parser loop, growing a `Vec` until the allocator aborted the
  process (exit 134) before a single command ran. Pasting a line out of a web page was enough.
- An unmatched `(` made `brush-parser`'s PEG retry an exponential number of alternatives: 20 of
  them took 0.64 s, 25 took 15.9 s, 30 never finished, all at 100% CPU.

Both are fixed. Their reproducers moved to `fuzz/seeds/` and are replayed on every `cargo test`,
and `fuzz/known/` — the directory for fuzzer findings that are *still* open — is empty.

The first fix costs nothing: the token scanner and the word scanner now share one predicate, and
it is the shell's separator set (space, tab, carriage return; newline is an operator) rather than
Unicode's, so `echo a<NBSP>b` prints one word exactly as bash does. The second is a bound, and
with the pre-existing depth limit it is the only restriction on input shape worth knowing:

- **At most 16 unmatched openers**, refused as a syntax error with exit status 2. bash rejects
  every input this rejects, so nothing that used to parse stops parsing.
- **At most 100 levels of nesting**, counted across `(`, `{`, `[` and the arithmetic and
  substitution forms.

## License

MIT.
