# oslo

A POSIX shell in Rust that also speaks Lua, with a structured pipeline that scripts written before
it existed provably cannot reach. Linux only.

```sh
oslo                       # a prompt
oslo script.sh arg1        # run a shell script
oslo build.lua             # run a Lua script — same command, no flag
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
one line as Lua from a shell prompt; `!` does the reverse. Neither changes the mode.

```sh
=print(("x"):rep(40))      # one Lua line, from shell
```

## Structured pipelines

A command can produce two things: text for you, and rows for the next command. The pipe decides
which — **before anything runs**, by reading what each stage declares.

```
$ df | where 'free < 1e9' | sort-by free
filesystem  size  used  free   capacity  mounted
efivarfs    192K  169K  19K    91        /sys/firmware/efi/efivars
tmpfs       1.0M  0B    1.0M   0         /run/credentials/systemd-journald.service
```

The same rows, into a pipe, become plain complete text — no colour, no borders, no truncation, no
`4.2G` where a program wanted `4509715660`.

It works with programs that know nothing about oslo, which is the point:

```sh
kubectl get pods -o json | from json | where 'status.phase == "Running"' | cols name
cat /etc/passwd | parse '{user}:{x}:{uid}:{rest}' | where 'uid > 1000' | get user
ls -1 | lines | where 'line:match("%.rs$")' | length
ps | where 'cpu > 10' | sort-by cpu | first 5 | to json | jq .
```

Filters are **Lua**, not a dialect invented for the occasion — so the escape hatch (`each`) is the
same language as the filter, and there is no cliff:

```sh
ls | each 'print(name .. " is " .. size .. " bytes")'
```

Verbs: `where` `each` `cols` `get` `sort-by` `first` `last` `length` `to` `from` `lines` `parse`.
Producers: `df` `ps` `ls`, and anything you register yourself.

### Your POSIX scripts cannot reach any of it

Structure flows only between two stages that **both** declare they understand it, and every name
that can carry a declaration is one oslo invented. A script written before oslo existed cannot name
one, so every edge in it plans to bytes and takes the path it always took.

That is not a promise, it is a build failure: `tests/posix_stays_on_the_byte_path.rs` runs all 408
corpus scripts and requires zero structured edges. There is also no new pipe operator — `a |> b` is
already valid POSIX, so the operator would itself be the hazard.

The design, including where nushell went wrong and why this diverges, is in
`docs/research/dual-channel-pipe.md`.

## The prompt

Built from named segments with priorities, so a narrow terminal drops the least important piece
instead of truncating mid-word:

```lua
oslo.theme.styles["my.dir"] = "fg:#8be9fd bold"

oslo.prompt.left = {
  oslo.segment{ name = "user", priority = 20,
    render = function(ctx) return {{ text = ctx.user .. "@" .. ctx.host, style = "prompt.user" }} end },
  oslo.segment{ name = "dir", priority = 90,
    render = function(ctx) return {{ text = oslo.path.shorten(ctx.cwd, 3), style = "my.dir" }} end },
}
```

`ctx` carries what a segment cannot get cheaply itself — `status`, `ok`, `duration_ms`, `cwd`,
`branch`, `vimode`, `language`, `cols`, `jobs` — gathered once so five segments do not run `git`
five times.

Or hand the whole thing to another program, with a deadline so a hung tool can never hang your
shell:

```lua
oslo.prompt.left = { command = "starship", args = { "prompt", "--status=$status" },
                     timeout_ms = 200, async = true }
```

Or just use `$PS1`, with the full escape set — `\u \h \w \$ \t \A \d \j \! \[ \]` and octal.

## Interactive

- **Ghost suggestions** from history, per language, never crossing between them
- **A completion dropdown** with columns, per-kind info and frecency ranking
- **Matching that is a transform, not a prefix test** — `/u/s/b` reaches `/usr/share/bin`, `f-b`
  reaches `foo-bar`, and an exact match is never diluted with fuzzy noise
- **Prefix history search** on Up, which restores the line you were composing instead of blanking it
- **First-class vi mode** on fish's model: cursor shape says the mode, the prompt says it too, and
  both update the instant it changes
- **A right prompt**, drawn without the save/restore that multiplexers fight over
- **Syntax highlighting** that marks a command that does not exist

### Abbreviations

```sh
abbr gco 'git checkout'      # typing `gco ` leaves `git checkout ` in the buffer
abbr gc 'git commit -m "%"'  # `%` says where the cursor lands
```

Better than an alias for a shell that promises not to change what scripts see: the real command
lands in the buffer, in history and in the language column, and you watch it happen.

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
prevd        # back one
nextd        # forward again
cd -3        # three back
dirh         # the ring
```

`cd -` is a one-deep toggle and useless once you are three wrong turns out. `pushd`/`popd` stay
exactly as they were, because scripts depend on them.

## Your own tools

```lua
oslo.register_tool{
  name = "hosts", produces = "rows",
  rows = function(argv)
    return { { host = "alpha", ip = "10.0.0.1", port = 22 },
             { host = "beta",  ip = "192.168.1.5", port = 80 } }
  end,
}
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

## Configuration

`~/.config/oslo/config.lua`, and `~/.oslorc` for shell-syntax startup.

```lua
oslo.completion.max_rows = 12
oslo.suggest.accept = "ctrl-f"
oslo.vi.enabled = true
oslo.on.cd(function(dir) print("now in " .. dir) end)
oslo.on["command-not-found"](function(name)
  print(name .. " is not installed — try: pkg install " .. name)
end)
```

`oslo.capture`, `sh.df()`, `sh.ps()`, `sh.ls()`, `sh.stat()`, `oslo.path.*`, `oslo.json`,
`oslo.re`, hooks, and a `did you mean` drawn from the command index oslo already keeps.

## Building

```sh
make build        # debug
make verify       # fmt, line limits, README paths, tests, clippy, rustdoc — all of it
make install      # to /usr/local/bin
nix build         # static musl binary
```

Every `.rs` file is under 600 lines, enforced by `scripts/check-loc.sh`. Split by meaning, never by
line count.

## Known gaps

Reproducible against the binary, and all but the last are differences from bash.

- **`for ((;;))`** is a syntax error when the separators touch — write `for (( ; ; ))`. The cause is
  upstream in brush's tokenizer, which fuses the two `;` into the `;;` that ends a `case` item.
- **Process substitution** needs `/dev/fd`, so it fails in an initramfs that has not set it up.
  bash has the same dependency.
- **`coproc` and `select`** are refused by name rather than half-implemented.
- **A failing special builtin** does not exit a POSIX-mode shell, though a failed readonly
  assignment does.
- **Arrays are indexed only.** `declare -A` says so rather than pretending.
- **`shopt`** knows every bash option and switches `autocd` and `globstar`. The rest report the
  state oslo actually has and *fail* when asked for the other one — an error rather than a lie.
- **A structured tool cannot read the shell's own stdin.** `df | where …` works; `cat x.json |
  oslo -c 'from json | …'` does not, because structure is assembled inside one pipeline and that
  form puts the bytes on the shell's descriptor instead. Use `oslo -c 'cat x.json | from json | …'`.
- No `SECONDS`, `RANDOM`, `LINENO`, `/dev/tcp` or restricted mode.

## Licence

MIT.
