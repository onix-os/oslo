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
- **Matching that is a transform, not a prefix test** — `/u/s/b` reaches `/usr/share/bin`, `f-b`
  reaches `foo-bar`, `gco` reaches `git checkout`, and each looser pass runs only when the stricter
  one found nothing, so an exact match is never diluted
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
oslo.completion = { fuzzy = "smart" }   -- dropdown
oslo.suggest    = { fuzzy = "smart" }   -- the inline ghost
```

`off`, `tight`, `smart` or `loose` — how far your letters may scatter, capped at 1, 4 and 8
unmatched characters. `gco` reaches `git checkout` at `smart`, not at `tight`. Always the last pass,
so switching it on cannot push a candidate you actually prefixed down the list.

A fuzzy suggestion replaces what you typed rather than continuing it, so it is never plain grey
text — it gets a `⟶` marker and accepting it overwrites the line. You cannot mistake a replacement
for a continuation, which is the only way this could get you to run something unread.

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

`direnv`, built in. `.envrc`, `.env` and `.env.lua` are found by walking up, nearest ancestor wins,
and **leaving puts everything back** — including variables the file unset, and variables that had no
value before and end up with none again rather than with an empty one.

```sh
direnv allow      # trust this file, as it stands right now
direnv deny       # refuse this path, whatever it becomes
direnv status     # what is loaded, what was found, whether it is trusted
```

Nothing is read until allowed, because `git clone` then `cd` is otherwise arbitrary code execution.
Allowing hashes the file's **contents**, so editing it revokes the allowance; denying hashes only
the **path**, so a refusal survives every edit. Both take effect where you stand, not on the next
`cd`.

`.envrc` runs on oslo's own evaluator — no bash, no subprocess, no serialised diff in your
environment. No direnv stdlib either, because there is Lua: `use` and friends are whatever your
config says they are.

```lua
oslo.register_builtin("use", function(args)
  if args[2] == "flake" then oslo.set_var("IN_FLAKE", "1", true) end
  return 0
end)
oslo.register_builtin("export_alias", function(args)
  oslo.set_alias(args[2], args[3]); return 0
end)
```

`.env.lua` is the interesting one: a directory that sets a keybinding, or a red prompt because it is
production. What an rc file prints is grouped under it, repeats collapsed with a count.

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

Reproducible against the binary, and all but the last are differences from bash.

- **`for ((;;))`** is a syntax error when the separators touch — write `for (( ; ; ))`. The cause is
  upstream in brush's tokenizer, which fuses the two `;` into the `;;` that ends a `case` item.
- **Process substitution** needs `/dev/fd`, so it fails in an initramfs without it. So does bash.
- **`coproc` and `select`** are refused by name rather than half-implemented.
- **A failing special builtin** does not exit a POSIX-mode shell, though a failed readonly
  assignment does.
- **Arrays are indexed only.** `declare -A` says so rather than pretending.
- **`shopt`** switches `autocd` and `globstar`; the rest report the state oslo actually has and
  *fail* when asked for the other one — an error rather than a lie.
- **A structured tool cannot read the shell's own stdin.** `df | where …` works; `cat x.json |
  oslo -c 'from json | …'` does not — structure is assembled inside one pipeline. Use
  `oslo -c 'cat x.json | from json | …'`.
- No `SECONDS`, `RANDOM`, `LINENO`, `/dev/tcp` or restricted mode.

## Licence

MIT.
