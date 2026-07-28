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
```

Available: `rush.exec`, `rush.get_var`, `rush.set_var`, `rush.get_pwd`, `rush.set_alias`,
`rush.get_alias`, `rush.set_prompt`, `rush.set_right_prompt`, `rush.register_builtin`.

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

Two suites, deliberately different in kind:

- `tests/posix_shell_tests.rs` — in-process: build an AST, evaluate it, inspect `Environment`.
- `tests/shell_behavior_tests.rs` — end-to-end: spawn the real binary and assert on stdout and
  exit status.

The second exists because the first cannot see a whole class of defect. A redirection dropped
during AST conversion, or an exit status never propagated to `main`, leaves the environment
looking perfectly correct while the shell is visibly broken.

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

- Parameter expansion does not run inside unquoted here-document bodies.
- `case` fallthrough (`;&`, `;;&`) is parsed but behaves as `;;`.
- Process substitution (`<(cmd)`, `>(cmd)`) is not supported.
- `[[ str =~ regex ]]` is rejected rather than approximated — there is no regex engine.
- Arithmetic supports `+ - * / % ( )` only; other operators are rejected.
- `((expr))`, `for ((;;))`, `coproc` and `select` are rejected by name, with exit status 2.
- `-e` and `-x` are accepted on the command line and recorded, but not yet honoured.
- Job control is minimal: `&` works, but there are no `jobs` / `fg` / `bg` builtins.
- `rush.register_builtin` records the name but the Lua callback is not invoked.
- The REPL has no multi-line continuation — an unterminated `for` is a syntax error, not a prompt.

## License

MIT.
