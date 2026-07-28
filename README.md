# rush

A POSIX-compatible shell in Rust, with Lua scripting and fish-style interactive features.

```sh
rush                      # interactive REPL
rush script.sh arg1 arg2  # run a script
rush -c 'echo hello'      # run a command
rush --lua-script init.lua
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
```

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
main.rs          REPL, -c, script and --lua-script entry points
lexer/           hand-written POSIX scanner
parser/
  brush_adapter  brush-parser AST -> rush AST  (the path actually taken)
  grammar.rs     hand-written recursive-descent parser (fallback)
ast/             the shared AST
expand/          parameter expansion, globbing, IFS splitting, arithmetic
exec/            fork/exec, pipelines, redirections, job control
env/             variables, scopes, aliases, functions, builtins
lua/             mlua bindings
interactive/     rustyline helper: completion, highlighting, dropdown, prompt
```

Parsing goes through [`brush-parser`](https://crates.io/crates/brush-parser), a spec-compliant
bash parser, and `brush_adapter` translates its AST into rush's simpler one. The hand-written
lexer and parser remain as a fallback for input brush rejects.

## Known gaps

- Parameter expansion does not run inside unquoted here-document bodies.
- `case` fallthrough (`;&`, `;;&`) is parsed but behaves as `;;`.
- Process substitution (`<(cmd)`, `>(cmd)`) is not supported.
- `[[ str =~ regex ]]` is rejected rather than approximated — there is no regex engine.
- Arithmetic supports `+ - * / % ( )` only; other operators are rejected.
- Job control is minimal: `&` works, but there are no `jobs` / `fg` / `bg` builtins.
- `rush.register_builtin` records the name but the Lua callback is not invoked.
- The REPL has no multi-line continuation — an unterminated `for` is a syntax error, not a prompt.

## License

MIT.
