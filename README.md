# oslo

A POSIX shell in Rust that also speaks Lua, with a structured pipeline that scripts written before
it existed provably cannot reach. Linux only.

```sh
oslo                       # a prompt
oslo script.sh arg1        # run a shell script
oslo build.lua             # a Lua script — same command, no flag
oslo -c 'echo hello'       # run a command
```

---

## Two languages, one prompt

Shift+Tab switches between shell and Lua **in place** — your line, your cursor and your history stay
where they are.

```
bresilla@tron | I | sh  ❯ ls -la                    ❮  (develop)  ~/src/oslo
bresilla@tron | I | lua ❯ for _, f in ipairs(sh.ls(".")) do print(f.name) end
```

Each language keeps its own history, suggestions, completion and syntax colouring. A `=` prefix runs
one line as Lua from a shell prompt (`=print(("x"):rep(40))`); `!` does the reverse. Neither changes
the mode.

## Structured pipelines

A command produces two things: text for you, rows for the next command. The pipe decides which
**before anything runs**, by reading what each stage declares.

```
$ df | where 'free < 1e9' | sort-by free
filesystem  size  used  free   capacity  mounted
efivarfs    192K  169K  19K    91        /sys/firmware/efi/efivars
tmpfs       1.0M  0B    1.0M   0         /run/credentials/systemd-journald.service
```

Into a pipe, the same rows become plain complete text — no colour, no borders, no `4.2G` where a
program wanted `4509715660`. It works with programs that know nothing about oslo:

```sh
kubectl get pods -o json | from json | where 'status.phase == "Running"' | cols name
cat /etc/passwd | parse '{user}:{x}:{uid}:{rest}' | where 'uid > 1000' | get user
ps | where 'cpu > 10' | sort-by cpu | first 5 | to json | jq .
```

Filters are **Lua**, not a dialect invented for the occasion, so the escape hatch is the same
language as the filter: `ls | each 'print(name .. " is " .. size)'`.

Verbs: `where` `each` `cols` `get` `sort-by` `first` `last` `length` `to` `from` `lines` `parse`.
Producers: `df` `ps` `ls`, and anything you register yourself.

### Your POSIX scripts cannot reach any of it

Structure flows only between two stages that **both** declare they understand it, and every name
that can carry a declaration is one oslo invented. A script written before oslo existed cannot name
one, so every edge in it plans to bytes.

That is not a promise, it is a build failure: `tests/posix_stays_on_the_byte_path.rs` runs all 408
corpus scripts and requires zero structured edges. There is no new pipe operator either — `a |> b`
is already valid POSIX, so the operator would itself be the hazard. Design:
`docs/research/dual-channel-pipe.md`.

## The prompt

Named segments with priorities, so a narrow terminal drops the least important piece rather than
truncating mid-word:

```lua
oslo.theme.styles["my.dir"] = "fg:#8be9fd bold"

oslo.prompt.left = {
  oslo.segment{ name = "dir", priority = 90,
    render = function(ctx) return {{ text = oslo.path.shorten(ctx.cwd, 3), style = "my.dir" }} end },
}
```

`ctx` carries `status`, `ok`, `duration_ms`, `cwd`, `branch`, `vimode`, `language`, `cols`, `jobs`
— gathered once so five segments do not run `git` five times.

Or hand it to another program, with a deadline so a hung tool cannot hang your shell:

```lua
oslo.prompt.left = { command = "starship", args = { "prompt", "--status=$status" },
                     timeout_ms = 200, async = true }
```

Or just use `$PS1`, with the full escape set — `\u \h \w \$ \t \A \d \j \! \[ \]` and octal.

## Interactive

- **Ghost suggestions** from history, per language, never crossing between them, and answering for
  the directory you are standing in
- **A completion dropdown** with columns, per-kind info and frecency ranking
- **Matching that is a transform, not a prefix test** — in the dropdown, `/u/s/b` reaches
  `/usr/share/bin`, `f-b` reaches `foo-bar` and `gco` reaches `git checkout`, each looser pass
  running only when the stricter one found nothing, so an exact match is never diluted
- **Prefix history search** on Up, which restores the line you were composing instead of blanking it
- **First-class vi mode** on fish's model: cursor shape says the mode, the prompt says it too
- **A right prompt**, drawn without the save/restore that multiplexers fight over
- **Syntax highlighting** that marks a command that does not exist

### Abbreviations

```sh
abbr gco 'git checkout'      # typing `gco ` leaves `git checkout ` in the buffer
abbr gc 'git commit -m "%"'  # `%` says where the cursor lands
```

Better than an alias for a shell that promises not to change what scripts see: the real command
lands in the buffer, in history and in the log, and you watch it happen.

### Keys that run your code

```lua
oslo.keys["ctrl-s"] = function(line) return "sudo " .. line.text end
oslo.keys["alt-u"]  = function(line)
  return { text = line.text .. " [" .. line.word .. "]", cursor = 0 }
end
```

Data in, data out — the handler is told about the line and answers with what it should become.

### Where you have been

```sh
cd -         # back one
cd -3        # three back
cd root      # the top of the git worktree, found without running git
cd oslo      # somewhere you have been, best match wins
dirh         # the ring, and the numbers `cd -N` takes
```

`cd -` is a one-deep toggle, useless three wrong turns out, so every move is recorded and `cd -N`
reaches any of them. `pushd`/`popd` are untouched — scripts depend on them.

**POSIX first, always.** A real directory, a `-`, a `-P` resolve exactly as before; the frecency
jump is reached only once `cd` has already failed to find the argument as a path, and only in an
interactive shell.

Ranking is not zoxide's. Its four loudest bugs are one defect — keywords filter but take no part in
the score, so the most-visited candidate wins however badly it matched. Here *match quality is the
primary key* and frecency only orders equal matches, so `cd rust` cannot land in `prust`. Each of
the four is a named passing test.

### What it remembers

A second database, beside the history one, recording where you go and what you run there:

| | |
|---|---|
| `dir` | path, visits, last visit, dwell time, git worktree root |
| `run` | the line, the directory, exit status, how many times, total and worst duration |

Which makes the ghost suggestion answer differently depending on where you are standing:

```sh
~/work/alpha ❯ cargo run --ex⏎     # → cargo run --example xyz
~/work/beta  ❯ cargo run --ex⏎     # → cargo run --example abc
```

The exact directory is asked first, then the worktree, then flat history. A line that never once
succeeded is never offered, so a typo stops haunting you.

It is `0600`, tightened before the first statement so the write-ahead log is born private too. A
line beginning with a space is not recorded, secrets are reduced to the command name, and **a
non-interactive shell never opens it at all**. `history -c` clears the lines and keeps the
directories: "forget what I typed" is not "forget where I work".

No daemon. The read is a B-tree range scan rather than a `LIKE` — 13 µs against 25,000 rows — which
is what makes a cache unnecessary, and a cache stale between two terminals impossible.

### Fuzzy matching

```lua
oslo.completion = { fuzzy = "smart" }   -- off / tight / smart / loose
```

`off`, `tight`, `smart` or `loose` — how far your letters may scatter, capped at 1, 4 and 8
unmatched characters. `gco` reaches `git checkout` at `smart`, not at `tight`. Always the last pass,
so switching it on cannot push a candidate you actually prefixed down the list.

**In the dropdown only.** The inline ghost suggestion stays a strict continuation of what you typed,
because that is the only thing it can honestly be: the editor draws a hint as text appended after
the cursor, so a suggestion that *replaces* your line cannot be shown as one without lying about
what pressing a key will do.

## Your own tools

```lua
oslo.register_tool{ name = "hosts", produces = "rows",
  rows = function(argv) return { { host = "alpha", ip = "10.0.0.1" } } end }
```

```
$ hosts | where 'ip:match("^10%.")' | cols host port
```

A tool says what its rows *are*. The shell decides how they are drawn — and when the next stage
wants rows, nothing is drawn at all.

## The terminal knows what is happening

`OSC 133` semantic marks (so a multiplexer can fold command output), `OSC 7` working directory,
`OSC 0` title, `OSC 8` hyperlinks, `OSC 52` clipboard over SSH, and desktop notifications for
commands that outlive a threshold. A `copy` builtin puts text on the clipboard through the terminal,
so it works over SSH with no X or Wayland helper installed.

## POSIX, where it counts

The language is the real thing: pipelines, redirections including heredocs and here-strings, all the
control flow, functions, `${var:-d}` and the rest of parameter expansion, arithmetic, globbing,
field splitting, job control with proper process groups and `tcsetpgrp`.

Correctness is measured rather than asserted. 408 scripts in `tests/corpus` run under both oslo and
bash and are compared byte for byte, with known differences listed in
`tests/differential/expected_fail.rs` as a two-way ratchet — the suite fails if a listed case starts
passing, so a stale entry cannot survive. 999 unit tests alongside.

## Directory environments

A `.env.lua` in a project, found by walking up from where you are, nearest ancestor wins. **Leaving
puts everything back** — including variables the file unset, and variables that had no value before
and end up with none again rather than with an empty one.

```lua
-- .env.lua
oslo.set_var("DATABASE_URL", "postgres://localhost/app_dev")
oslo.set_alias("t", "cargo test")
oslo.direnv.path_add("./bin")   -- prepended, idempotent, gone when you leave
oslo.set_prompt(function() return "PRODUCTION> " end)
```

Variables, aliases, `PATH` and the prompt all come back on the way out — one that was shell-local
before the directory exported it comes back *local*, not deleted. Keybindings deliberately do not:
a key meaning different things in different directories, with nothing on screen to say so, is worse
than not having the feature.

```sh
direnv allow      # trust this file, as it stands right now
direnv deny       # refuse this path, whatever it becomes
direnv status     # what is loaded, what was found, whether it is trusted
```

Nothing is read until allowed, because `git clone` then `cd` is otherwise arbitrary code execution.
Allowing hashes the file's **contents**, so editing it revokes the allowance; denying hashes only
the **path**, and survives every edit. Both take effect where you stand, not on the next `cd`.

```lua
-- a Nix flake's dev shell, without entering one
oslo.direnv.nix_develop()
```

direnv's `use flake` is `eval "$(nix print-dev-env …)"` — a hundred kilobytes of generated bash.
This reads `--json` instead. The catch, and why it is built in rather than left as a recipe: the two
forms do **not** carry the same variables. `nix` withholds `HOME` and four others from the shell
form because setting them would wreck the shell you are in — `HOME` in a derivation is
`/homeless-shelter` — and `--json` applies no such filter.

**Lua, and only Lua.** `.envrc` and `.env` were both supported for a while and both are gone:
`.envrc` meant either shipping direnv's 1,400-line stdlib or failing on every real file that says
`use flake`, and `.env` is a second grammar for what one Lua line already says. There is no bash
subprocess and no serialised diff in your environment — oslo *is* the shell.

Because it is Lua, a directory can set more than variables — an alias for its test command, say —
and **aliases are restored on the way out too**, so a project's `t` cannot follow you into the next
one and run the wrong tests. Whatever the file prints is grouped under it, repeats collapsed with a
count.

## Configuration

`~/.config/oslo/config.lua`. One file, one language, one place — there is no shell-syntax config.
(`$ENV` and `$PS1` still work: those are POSIX's, not oslo's, and a `/bin/sh` does not get to
refuse them.)

```lua
oslo.completion.max_rows = 12
oslo.suggest.accept = "ctrl-f"
oslo.vi.enabled = true
oslo.on.cd(function(dir) print("now in " .. dir) end)
oslo.on["command-not-found"](function(name) print(name .. " is not installed") end)
```

`oslo.capture`, `sh.df()`, `sh.ps()`, `sh.ls()`, `sh.stat()`, `oslo.path.*`, `oslo.json`, `oslo.re`,
hooks, and a `did you mean` drawn from the command index oslo already keeps.

## Building

```sh
make build        # debug
make verify       # fmt, line limits, README paths, tests, clippy, rustdoc — all of it
make install      # to /usr/local/bin
nix build         # static musl binary
```

Every `.rs` file is under 600 lines, enforced by `scripts/check-loc.sh`.

## Known gaps

`for ((;;))` with touching separators, process substitution without `/dev/fd`, `coproc`, `select`,
associative arrays, and a structured tool reading the shell's own stdin. Each one is listed with its
cause and its workaround in [docs/known-gaps.md](docs/known-gaps.md), most of them pinned by a test
that fails if the gap ever closes.

## Licence

MIT.
