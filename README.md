# rush

A POSIX-compatible shell in Rust, with Lua scripting and fish-style interactive features.

```sh
rush                      # interactive REPL when stdin is a terminal, else read stdin
rush script.sh arg1 arg2  # run a script
rush -c 'echo hello'      # run a command
rush -s < script.sh       # read the program from standard input
rush --lua-script init.lua
rush --help               # -c -s -i -l -e -x --version --help --
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

Builtins: `cd` `pwd` `echo` `export` `unset` `set` `shift` `exit` `break` `continue` `return`
`alias` `unalias` `type` `eval` `source` `.` `read` `local` `pushd` `popd` `dirs` `readonly`
`test` `[` `[[` `trap` `umask` `wait` `kill` `true` `false`.

## Lua

`~/.config/rush/init.lua` is loaded at startup.

```lua
rush.set_prompt(function()
  return rush.get_pwd() .. " ❯ "
end)

rush.set_alias("gs", "git status")
rush.exec("echo from lua")
rush.set_var("EDITOR", "nvim")

rush.register_builtin("hello", function(argv)
  print("hello, " .. (argv[2] or "world"))
  return 0
end)
```

Available: `rush.exec`, `rush.get_var`, `rush.set_var`, `rush.get_pwd`, `rush.set_alias`,
`rush.get_alias`, `rush.set_prompt`, `rush.register_builtin`.

`rush.register_builtin(name, fn)` makes `name` a builtin, ahead of `PATH` and overriding any
builtin of the same name. The callback receives argv as a table with `argv[1]` set to the
builtin's own name. Its return value is the exit status: no return value or `true` is 0, `false`
is 1, a number is that number. A Lua error is reported on stderr and the builtin exits 1.

Two limits worth knowing. The rest of the `rush.*` API is unavailable *inside* such a callback —
the shell state is already borrowed by the evaluator running it, so `rush.exec` and friends raise
an error there rather than deadlocking. And there is no right prompt: `rush.set_right_prompt`
existed but nothing ever drew what it returned, so it was removed rather than left as an API that
silently does nothing.

## Interactive

Syntax highlighting (valid commands green, unknown red), history hints, a completion dropdown
with descriptions, and a git-aware prompt. History is kept in `~/.rush_history`.

## Building

```sh
make build     # build the shell
make run       # build and run the REPL
make test      # run the test suite
make verify    # the full gate: fmt, loc, check, test, clippy, rustdoc
make install   # install to $PREFIX/bin (default ~/.local/bin)
```

The minimum supported Rust version is 1.88 (edition 2024 plus let-chains); CI checks it on every
push, alongside a verify matrix over Linux and macOS.

## Installing

```sh
make install                 # builds --release and installs to ~/.local/bin/rush
make install PREFIX=/usr/local   # or anywhere else
make uninstall               # removes it again (same PREFIX)
```

`PREFIX` defaults to `$HOME/.local`, so the binary lands in `$HOME/.local/bin` — make sure that
directory is on your `PATH`. `DESTDIR` is honoured for staged/packaging installs.

Check it works before going further:

```sh
~/.local/bin/rush -c 'echo ok'
```

### Using rush as your login shell

Read [Known gaps](#known-gaps) first. A login shell that cannot parse your startup files locks
you out of your own account, and the recovery path (another terminal, or single-user mode) is
not always available on a remote machine.

A shell may only be a login shell if it is listed in `/etc/shells`:

```sh
sudo make install PREFIX=/usr/local            # system-wide, so it survives $HOME being unmounted
echo /usr/local/bin/rush | sudo tee -a /etc/shells
chsh -s /usr/local/bin/rush
```

`chsh` refuses any path not in `/etc/shells`, which is the check that stops you from setting a
shell that does not exist. Log out and back in for the change to take effect.

Before you commit to it, run rush as a non-login shell for a while — start it from your current
shell, or set it as your terminal emulator's command — so that a breakage costs you a window
instead of a session.

To go back:

```sh
chsh -s /bin/bash
```

If you are already locked out, most systems let you log in over SSH with an explicit shell
(`ssh host -t /bin/bash -l`), or you can switch the entry back with `sudo chsh -s /bin/bash $USER`
from a rescue session.

## File length

**No source file may exceed 600 lines.** Enforced by `scripts/check-loc.sh`, which runs as part
of `make verify`; check it directly with `make check-loc`.

A file that outgrows the limit has almost always taken on a second responsibility, and the point
where it breaches is usually the seam to split along. Split modules are named for **what they
contain** — `redirects.rs`, `quoting.rs`, `conditionals.rs` — never for their position
(`part1.rs`, `helpers2.rs`), which tells a reader nothing about where to look.

## Testing

`cargo test` runs three kinds of suite, deliberately different in what they can see:

- **In-process** — `tests/posix_shell_tests.rs` builds an AST, evaluates it and inspects
  `Environment` directly. Fast, but blind to anything that only goes wrong in a real process.
- **End-to-end** — most of `tests/`: `tests/common/mod.rs` spawns the real binary with stdin from
  `/dev/null` under a timeout, and the test asserts on stdout, stderr and exit status. This exists
  because the first kind cannot see a whole class of defect: a redirection dropped during AST
  conversion, or an exit status never propagated to `main`, leaves the environment looking
  perfectly correct while the shell is visibly broken.
- **Differential** — `tests/differential_tests.rs` runs every script in `tests/corpus/` through
  both rush and bash and compares stdout and exit status (stderr by shape only, since diagnostic
  wording should differ). It is a ratchet: `tests/differential/expected_fail.rs` names each case
  rush still gets wrong along with the finding that explains it, and the suite fails both when an
  unlisted case diverges *and* when a listed case starts passing. Closing a bug means deleting a
  line there.

The line editor is covered without a pty: `RushHelper` is public, so `tests/interactive_tests.rs`
and `src/interactive/tests.rs` call `complete`, `hint`, `highlight` and `input_status` directly
against temporary directories.

## Architecture

```
main.rs          script, -c, stdin and REPL entry points
cli.rs           argv parsing for the binary
lexer/           hand-written POSIX scanner (word re-lexing for the adapter)
parser/
  brush_adapter  brush-parser AST -> rush AST  (the only parsing path)
ast/             the shared AST
expand/          parameter expansion, globbing, IFS splitting, arithmetic
exec/            fork/exec, pipelines, redirections, job control
env/             variables, scopes, aliases, functions, builtins
lua/             mlua bindings
interactive/     rustyline helper: completion, highlighting, dropdown, prompt
```

Parsing goes through [`brush-parser`](https://crates.io/crates/brush-parser), a spec-compliant
bash parser, and `brush_adapter` translates its AST into rush's simpler one. That is the only
path: input brush or the adapter rejects becomes a syntax error with a position, and the shell
exits 2. There used to be a second, hand-written parser used as a fallback, which had no
here-document support and therefore executed heredoc *bodies* as commands; it is gone. The
hand-written lexer stays — the adapter re-lexes brush's raw word text through it.

## Known gaps

Each of these is reproducible against the binary, and each has a corpus case in
`tests/differential/expected_fail.rs` unless noted.

- Parameter expansion does not run inside unquoted here-document bodies or here-strings; the text
  is passed through with quotes removed but `$v` left alone.
- Process substitution (`<(cmd)`, `>(cmd)`) is refused by name with exit status 2. Refusing is
  deliberate — silently dropping the argument made `diff <(a) <(b)` report false success — but the
  `/dev/fd/N` implementation is not written.
- `coproc` and `select` are likewise refused by name. Both need machinery (job control, and a
  prompt plus `REPLY`) out of proportion to how often scripts use them.
- Arrays are indexed only. `declare -A` reports that associative arrays are unsupported rather
  than pretending, and an operator applied to a whole array (`${a[@]:1}`, `${a[@]#pat}`) is
  rejected instead of evaluated.
- `for ((;;))` needs spaces: brush tokenizes the `;;` in the unspaced form as the `case`
  terminator, so write `for (( ; ; ))`.
- `unset -f` cannot remove a function, and assignment to a `readonly` variable succeeds when it
  should fail.
- `exec 3> file` closes fd 3 instead of leaving it open.
- A failing special builtin does not exit a POSIX-mode shell.
- No `shopt`, so `globstar` cannot be turned on; `**` behaves as `*`.
- No `SECONDS`, `RANDOM`, `LINENO`, `/dev/tcp`, restricted mode, or `vi`/`emacs` editing modes.
- History expansion (`!!`, `!$`, `^old^new`) applies to REPL lines only, not to `-c` or scripts —
  the same rule bash uses.

## License

MIT.
