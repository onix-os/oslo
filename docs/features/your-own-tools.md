# Your own tools

Three ways to add a name to this shell: a **tool** that produces rows for the structured pipeline, a
**builtin** that is a builtin in every sense the shell has — both written in Lua — and a **shell
function kept in its own file**, read the first time it is called. They land in three different
tables, and which one answers for a name something else already owns is decided by which table is
consulted, not by any rule about precedence between them.

<!-- demo:begin -->
[![your-own-tools demo](https://asciinema.org/a/1262756.svg)](https://asciinema.org/a/1262756)
<!-- demo:end -->

## How it works

A command word is resolved twice by two entirely separate mechanisms, and that is the fact the whole
document hangs on. If the pipeline's plan gives some edge structured rows, the stage names are
looked up in the tool registry and *nowhere else*. Otherwise the ordinary POSIX command search runs,
and the two Lua-supplied things it can find are a registered builtin and a file of functions.

```
  a command word
     │
     ├─ does the pipeline's plan give any edge rows?   see structured-pipelines.md
     │     │
     │     yes ─► data::custom ─► the Lua `rows` function, called with argv
     │            the ONLY lookup for that stage: no alias, no function,
     │            no builtin, no $PATH
     no
     ▼
  POSIX command search — exec::simple::run_command_word
     alias ─► function ─► builtin ─► $PATH ─► not found
                 ▲           ▲                   │
                 │           └ register_builtin   │
                 └ a NAME.sh already loaded       ▼
                                        autoload::try_call
                                        read NAME.sh, call NAME
     under --posix only, a special builtin is tried ahead of the function
```

### A tool is a source of rows, and only that

```lua
oslo.register_tool{
  name     = "mounts",
  accepts  = "nothing",   -- nothing | bytes | rows | any   (default: nothing)
  produces = "rows",      -- the same four                  (default: rows)
  rows     = function(argv, input) return { { a = 1 }, { a = 2 } } end,
}
```

`name` must be a string and `rows` must be a function, or the call raises. A shape that is not one
of the four raises too, naming the four — a typo in `produces` would otherwise register a tool that
silently never passes anything on.

Registration writes to two tables: `data::tool::register` records the shapes, which is all the
planner ever reads, and `data::custom::register` holds the closure, which is what runs. The split is
there so that the pipeline can ask "is there a tool called this" without reaching up into the Lua
API — the API sits above the shell, and the shell asking it a question would invert that.

**The handler is given its argv and its input.** `argv[1]` is the tool's own name and the rest are
its words. The second argument is what the stage before it produced: a list of rows for a tool that
declared `accepts = "rows"`, and `nil` for one that declared `nothing` or that has no neighbour. So
a Lua tool can consume as well as produce —

```lua
oslo.register_tool{ name = "count", accepts = "rows",
  rows = function(argv, input) return { { rows = #input } } end }
```

**A third argument for a tool that declared `accepts = "bytes"`** — the raw stream, as a string:

```lua
oslo.register_tool{ name = "counted", accepts = "bytes",
  rows = function(argv, input, bytes)
    local n = 0
    for _ in bytes:gmatch("[^\n]+") do n = n + 1 end
    return { { lines = n } }
  end }
```

```
$ printf 'a\nb\nc\n' | counted | cols lines
lines
3
```

That shape was accepted by the validator and could not work: the planner routed the bytes here —
it reads standard input for a bytes-accepting tool at the head of a pipeline — and they were dropped
one call short of the handler, so `bytes` was always `nil`. It is `nil` for every other shape, by the
same rule `input` follows: given nothing and given no bytes are different questions.

Declaring `"bytes"` copies the whole stream into a Lua string, so a tool reading a 200 MB pipe costs
200 MB. A tool that wants to stream takes `rows` with `lines` in front of it instead.

`accepts` is still what the planner reads to decide the edge; what changed is that the input is then
handed over rather than dropped.

The returned value is read as a list of tables. Each table becomes one row, columns in the order the
table has them; a string, number, boolean or nil becomes the matching `Val`, a nested table becomes
a record, and a row that ends up with no columns is dropped.

### A worked example

`$XDG_CONFIG_HOME/oslo/init.lua`:

```lua
oslo.register_tool{
  name     = "mounts",
  produces = "rows",
  rows = function(argv)
    local rows = {}
    for line in oslo.fs.lines("/proc/mounts") do
      local device, point, kind, opts = line:match("^(%S+)%s+(%S+)%s+(%S+)%s+(%S+)")
      if device then
        rows[#rows + 1] = { device = device, point = point, kind = kind,
                            rw = opts:match("^rw") ~= nil }
      end
    end
    return rows
  end,
}
```

```
$ mounts | where 'kind == "ext4"' | cols point device rw
point                device          rw
/                    /dev/nvme0n1p2  true
/home                /dev/sdb3       true
/home/bresilla/data  /dev/sdb2       true

$ mounts | sort-by point | first 2 | cols point kind
point      kind
/          ext4
/boot/efi  vfat
```

`oslo.fs.lines` is used rather than `cat /proc/mounts`, and that is not a stylistic choice: see the
limits below. `oslo.tools()` answers with the names registered so far, sorted, which is the only way
to tell a tool that failed to register from one whose name you misspelled.

### A registered builtin

```lua
oslo.register_builtin("hi", function(argv)
  print("hi " .. (argv[2] or "there"))
  return 0
end)
```

This goes into the one builtin table `is_builtin`, the dispatcher, `type`, `command -v` and
completion all read, so `type hi` answers *hi is a shell builtin* and `command -v hi` answers `hi`.
`argv[1]` is the builtin's own name, as it is for every builtin written in Rust — the dispatcher
recovers the closure from it. Redirections and pipelines work because nothing about the builtin is
special: `hi world > /tmp/hi.txt` writes the file and `hi there | tr a-z A-Z` prints `HI THERE`.

What the callback returns becomes the exit status:

| returned | status |
|---|---:|
| `true`, `nil`, or nothing | 0 |
| `false` | 1 |
| a number | that number |
| a string | parsed as one, 0 if it does not parse |
| an error | the message on stderr, and 1 |

Last registration wins, and the table it wins in is the one the shipped builtins are in — so
`oslo.register_builtin("date", …)` really does replace `date` for that shell. The name still loses
to a shell function, and `\date` still gets the program on `$PATH`.

### Functions kept one to a file

`~/.config/oslo/functions/NAME.sh` defines `NAME` and is not read until something calls it.
`$XDG_CONFIG_HOME` is honoured, the name must be a plain filename (`../evil`, `a/b` and `.hidden`
resolve to nothing), and the file is read through the ordinary `source` builtin — same parser, same
alias expansion, same nesting limit as a file you sourced by hand.

**The lookup happens after the `$PATH` search has already failed**, so autoloading adds names and
never changes one. A file called `ls.sh` is dead weight; `ls` has resolved to the program long
before. A file that exists but does not define the function it is named for is reported once —
`oslo: …/empty.sh: does not define empty` — and the command ends 127 without also claiming it was
not found, because it plainly was.

Unlike the other two, this one is not configuration: it lives in the shell rather than in the Lua
layer, so it works in `oslo -c 'gitroot'` and in a `#!/bin/oslo` script, neither of which reads a
config at all. At a prompt `\gitroot` skips it, since `\cmd` means "the program and nothing else";
in a script `\cmd` keeps its POSIX meaning and the function is still loaded.

## What makes it different

bash has loadable builtins, but a new one is C compiled to a shared object and loaded with
`enable -f`. Here it is a function in the config file, and the config file is already Lua.

fish reads a `functions/` directory the same way, and this is copied from it, arithmetic included: a
snippet defining twenty functions costs twenty definitions on every shell start — including the
hundred short-lived ones a build spawns — where an autoloaded one costs a `stat(2)` on the call that
needs it. The difference is the shadowing rule. fish lets an autoloaded function override a command;
oslo reads the file only after `$PATH` has been searched, because a shell that promises scripts see
POSIX behaviour cannot have a file on disk quietly redefining `test`.

`oslo.register_tool` has no counterpart in bash, zsh or fish for the simple reason that there is
nothing for it to join: a command in those shells produces bytes, so extending them means writing a
program that prints. Here a tool declares a shape and the planner reads it, which is why a Lua tool
composes with `where` and `sort-by` without either side knowing about the other.

### What a locked surface *can* do

A builtin and an answering hook cannot run a command, but they are handed words and asked questions
about them — and the two rules the shell applies to a word before it runs one are now callable:

```lua
oslo.word.braces("src/{a,b}.rs")      --> { "src/a.rs", "src/b.rs" }
oslo.word.matches("main.rs", "*.rs")  --> true
oslo.proc.parse("cat 'a b' | grep x") --> { {link="first", kind="simple", argv={"cat","a b"}, redirects=0},
                                      --     {link="|",     kind="simple", argv={"grep","x"},  redirects=0} }
```

All three are pure functions of a string, so they work everywhere — including the two places
`oslo.run` raises. That is most of the reason they exist: **the surface that cannot run a command is
the one that most needs to understand one.**

Written by hand, each is subtly wrong. Splitting a line on whitespace breaks `cat 'a b'`, `echo
$HOME` and `a | b`. Matching a glob with `string.find` after rewriting `*` to `.*` is a different
language — `.` is a metacharacter in a Lua pattern and is not one in a shell glob, so `a?txt`
against `a.txt` answers wrongly in both directions. Brace expansion by hand gets nesting and
`{1..9}` wrong.

`oslo.proc.parse` answers `nil` for a line that does not parse — the shell is about to report the
syntax error better than a caller could. Its `redirects` is a **count**, not a list: it is the same
payload `pre-cmd` already receives, with a door on it.

## Configuration

There are no settings — the whole surface is three calls, made from
`$XDG_CONFIG_HOME/oslo/init.lua` or any `conf.d/*.lua` beside it, plus one directory.

```lua
oslo.register_tool{ name = "mounts", accepts = "nothing", produces = "rows",
                    rows = function(argv) return { { a = 1 } } end }
oslo.tools()                              --> { "mounts" }

oslo.register_builtin("hi", function(argv) print(argv[2]) return 0 end)
```

```sh
$ cat ~/.config/oslo/functions/gitroot.sh
gitroot() { git rev-parse --show-toplevel; }
```

## What it cannot do

Two entries left this list rather than being fixed in it, and both are worth knowing if you read an
older copy: a tool **is** a command on its own now (`mounts` alone prints its rows, where it used to
be *command not found*), and a tool **does** receive its input.

- **A byte consumer breaks it too.** `mounts | grep ext4` is *command not found*, because that edge
  is bytes and the whole plan is thrown away. There is no byte prefix in reverse: everything before
  the first tool may be ordinary commands, nothing after the last one may be.
- **A stage after the last tool is worse than refused.** `mounts | to json | head -3` plans as
  structured, runs, and prints the whole JSON — `to` writes it with `println!` — before it meets
  `head`, which is not a tool. At that point the run gives up and the *whole* pipeline is retried on
  the byte path. The output has already been written, and the retry adds *to: command not found* and
  *mounts: command not found* underneath it.
- **A redirection removes both stages.** `mounts | where rw > /tmp/x` reports *mounts: command not
  found* and *where: command not found*, the built-in verb included. A redirection means bytes were
  asked for somewhere specific, and the tools have no byte form to fall back to.
- **Twenty-eight calls fail inside a tool or a registered builtin** — the ones that reach the
  shell's own state, listed below. They run while the shell holds that state, so those raise *shell
  state is busy*. `io.popen` is refused separately, because it would run its argument in `/bin/sh`
  rather than this shell. To fold an external program's output in, put it before the first tool:
  `kubectl get pods -o json | from json | where …`.

  **This entry used to say "nothing … may reach the shell", and that was wrong in a way that
  mattered.** Read as written it means a builtin can do nothing, and a survey of the whole Lua
  surface reached exactly that conclusion — proposing lock-free substitutes for capabilities that
  were already reachable. `borrow_env`, the only thing that takes the lock, is in **seven of
  forty-six** files under `crates/oslo-runtime/src/lua/api/`. Everything else works.

  | reaches the shell — raises | free to use |
  |---|---|
  | `oslo.run`, `oslo.pipe`, `oslo.lines`, `sh.*` | `oslo.fs.*`, `oslo.path.*` |
  | `oslo.env.{get,set,unset,all,alias,set_alias}` | `oslo.json.*`, `oslo.re.*`, `oslo.hash.*`, `oslo.hex.*` |
  | `oslo.env.{path,path_add,path_remove,has_path}` | `oslo.db.*`, `oslo.state.*` |
  | `oslo.proc.{exec,capture,status}` | `oslo.git.*`, `oslo.history.*` |
  | `oslo.sys.{cd,user,interactive,login}` | `oslo.ui.*`, `oslo.term.*`, `oslo.messages.*` |
  | `oslo.opts.{get,set}`, `oslo.source` | `oslo.spawn`, `oslo.after`, `oslo.every` |
  | `oslo.direnv.*`, `oslo.repair`, `oslo.register_builtin` | `oslo.fs.watch`, and all of Lua |

  `oslo.proc.status` being on the left is the one that hurts: it is `$?`, so a builtin cannot see
  the status of the command before it — by that route or by the Lua global, which `try_lock`s and
  answers `nil`. `tests/lua_api_surface_tests.rs` walks both columns and fails if either moves.
- **Tools are invisible to everything that answers questions about names.** `type mounts` says not
  found, `command -v mounts` exits 1, and completion does not offer it. Only the pipeline planner
  knows the registry exists.
- **Inside a rows pipeline the tool registry outranks the entire command search.** A tool named
  `date` wins over a shell function, over the `date` builtin and over `/usr/bin/date` — but only
  there. `date` alone still runs whichever of those the search finds, so one name can mean two
  things depending on whether it is in a structured pipeline.
- **A word that is not a plain literal is never a tool.** `\date | cols d` and `$cmd | cols d` plan
  as bytes, because a name that comes out of an expansion is not known until the command runs.
- **Only the last stage's status is the pipeline's.** A tool that raises reports 1, but
  `bad | cols a` still ends 0 — `cols` ran and succeeded. `PIPESTATUS` has the whole vector.
- **A tool's error is reported without its name**: `oslo: 4: nope` — the Lua line, not the tool.
- **Tools and Lua builtins do not exist outside an interactive shell.** The config is read only by
  the REPL, so `oslo -c 'mounts | cols point'` is *command not found*. Autoloaded functions are the
  exception and work everywhere.
- **An autoloaded file is shell, not Lua.** `path_for` looks for `NAME.sh` and nothing else.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-runtime/src/lua/api/tool.rs` | `oslo.register_tool`, `oslo.tools`, `shape_of`, `run_rows`, `records_of` |
| `crates/oslo-shell/src/data/custom.rs` | `register`, `rows_of` — the closure table the pipeline reads |
| `crates/oslo-shell/src/data/tool.rs` | `Tool`, `register`, `lookup`, `any_registered` — the declarations |
| `crates/oslo-shell/src/data/tools/mod.rs` | `run_tool`, which asks `custom` before its own `match` |
| `crates/oslo-shell/src/exec/pipeline/structured.rs` | `structured_sinks`, `run`, `simple_command_name` |
| `crates/oslo-runtime/src/lua/api/mod.rs` | `oslo.register_builtin` |
| `crates/oslo-runtime/src/lua/engine.rs` | `call_lua_builtin`, `status_from_lua`, `BUILTIN_KEY_PREFIX` |
| `crates/oslo-shell/src/env/scope/registry.rs` | `BuiltinRegistry`, `register_dynamic`, `invoke_dynamic_builtin` |
| `crates/oslo-shell/src/exec/simple/autoload.rs` | `path_for`, `load`, `try_call` |
| `crates/oslo-shell/src/exec/simple.rs` | `run_command_word`, `run_program` — where autoload sits in the search |
| `crates/oslo-shell/src/exec/simple/escape.rs` | `Escape`, and why `\cmd` skips the autoload step |
| `crates/oslo-runtime/src/startup/lua_init.rs` | `config_paths`, `config_files` — where a config is read from |
