# Structured pipelines

A command in oslo produces two things: text for a person, and rows for the next command. The pipe
decides which one an edge carries **before any stage runs**, by reading what each stage declares —
never by looking at the bytes, because by the time output exists the producer has already chosen a
form and paid to render it.

<!-- demo:begin -->
[![structured-pipelines demo](https://asciinema.org/a/1262749.svg)](https://asciinema.org/a/1262749)
<!-- demo:end -->

## How it works

Every command word is looked up in a registry of declarations. A declaration is two shapes: what
the command *accepts* from the stage before it, and what it *produces* for the stage after. A name
nobody registered declares nothing, which is the answer for every external command and every
builtin oslo has today.

The pipeline is then walked right to left. The last stage is `Print` when its stdout is a terminal
and nothing redirects it, and `Text` otherwise. Every earlier edge is decided on its own:

```
  df | where 'free < 1e9' | sort-by free                stdout: a terminal
  ─┬   ────────┬─────────   ─────┬──────
   │           │                 │
   │           │                 └─ last, terminal, not redirected ──▶ Sink::Print
   │           └── rows→rows, in-process, no redirection ───────────▶ Sink::Rows
   └────────────── nothing→rows, in-process, no redirection ────────▶ Sink::Rows

  for one edge   producer ──▶ consumer
        ┌── producer.produces gives rows ?      no ──▶ Sink::Text
        ├── consumer.accepts takes rows ?       no ──▶ Sink::Text
        ├── both stages run in this process ?   no ──▶ Sink::Text
        └── neither stage redirected ?          no ──▶ Sink::Text
                              all four yes ──▶ Sink::Rows, and the audit counter += 1
```

If no edge came out `Rows`, the plan is thrown away and the pipeline runs on the byte path it
always ran on — one forked process per stage, bytes on every descriptor. If some edge did, the
whole pipeline is run in this process instead, with the rows handed over by move: there is no
descriptor between two structured stages, so there is nothing to serialise and no format to get
wrong.

A pipeline may begin with ordinary commands. Everything up to the first registered tool runs on the
byte path exactly as it always did, with stdout pointed at a pipe, and its captured output is what
the first tool is given — which is how `kubectl … | from json | where …` works with a program that
knows nothing about oslo.

**And it may end with them.** Where the tools stop, what they made is rendered — the transport form,
never the drawn table — and handed to the rest of the pipeline as its standard input. That is how
`ls | first 2 | cat` and `ps | first 3 | to json | jq .` work. A tool that *prints* rather than
producing rows is carried the same way: whether a byte suffix exists is known before any stage runs,
so the tool half's own stdout is captured for its duration.

```
  ps | first 3 | to json | jq .
  └──── in this process, rows ───┘   └─ forked, on descriptors, reading what the tools wrote
```

Both halves report their own statuses, so `PIPESTATUS` and `pipefail` describe the pipeline that was
written rather than the halves it happened to run in.

The vocabulary is the whole of what can carry structure:

| name | accepts | produces |
| --- | --- | --- |
| `df` `ps` `ls` | nothing | rows |
| `lines` `parse` `from` | bytes | rows |
| `where` `each` `cols` `get` `sort-by` `first` `final` `length` | rows | rows |
| `to` | rows | bytes |

`cols` rather than `select`, because `select` is a bash keyword and oslo's parser refuses it as one.
`from json` rather than `from-json`, because the format is an argument and a format oslo learns
later then needs no new command name. `each` declares rows but produces none — it runs its Lua for
the side effect, so the pipeline ends there.

The producers' columns, as they actually come out:

| producer | columns |
| --- | --- |
| `df` | `filesystem` `size` `used` `free` `capacity` `mounted` |
| `ps` | `pid` `name` `cmdline` `is_kernel` |
| `ls` | `name` `size` `size_human` `is_dir` `mode` |

Sizes are a tagged scalar rather than the text `4.2G`, which is what makes `sort-by size` order by
bytes and `where 'free < 1e9'` arithmetic. There are two renderers and they were two from the first
commit: `render_display` for a person — human sizes, a header, aligned columns — and
`render_transport` for a program — plain, complete, untruncated, tab separated, one record per line,
sizes as their number of bytes. A drawn table can never reach a pipe, because the function that
draws it is not the function a pipe calls.

`df` on a terminal:

```
filesystem      size  used  free  capacity  mounted
tmpfs           12G   3.7M  12G   1         /run
/dev/nvme0n1p2  915G  220G  648G  26        /
```

The same rows into a pipe, with no header and no abbreviation:

```
tmpfs	12534099968	3887104	12530212864	1	/run
/dev/nvme0n1p2	982240026624	236755972096	695513538560	26	/
```

Filters are Lua, not a dialect invented for the occasion. The row's columns are bound as globals for
one evaluation, so `free < 1e9` reads the way it looks, and `row` is bound too for a column whose
name is not a Lua identifier. The expression is parsed once for the whole table, not once per row.
A row whose expression raises is dropped and the failure is reported once — keeping such rows would
be worse, because a filter that quietly passes everything when it breaks is how a pipeline ending in
`rm` removes the wrong thing.

### The same verbs, as functions

A pipeline is the only way to reach these from a *shell* line — and there is no pipeline inside
`oslo make`, inside a registered builtin, or inside a completion provider. So a recipe that wanted
rows sorted by a column wrote the sort again in Lua and got a different answer: `table.sort`
compares `"100"` below `"9"`, and `sort-by` does not.

```lua
local rows = oslo.rows.from_json(oslo.run{"docker","ps","--format","json", capture=true}.out)
local big  = oslo.rows.where(rows, "size > 1e9")
print(oslo.rows.render(oslo.rows.sort_by(big, "name"), "table"))
```

| call | answers |
|---|---|
| `oslo.rows.where(rows, expr)` | the rows the expression is true for, and a message if it broke |
| `oslo.rows.sort_by(rows, col)` / `.cols(rows, {…})` / `.get(rows, col)` | reshaped rows |
| `oslo.rows.first(rows, n)` / `.last(rows, n)` | an end of the table |
| `oslo.rows.length(rows)` | a **number**, not the one-row table the pipeline verb answers with |
| `oslo.rows.group_by(rows, col)` / `.count(rows)` / `.distinct(rows, [col])` / `.stats(rows, col)` | summaries |
| `oslo.rows.render(rows, "table"\|"text"\|"json")` | a string |
| `oslo.rows.lines(text)` / `.parse(text, pattern)` / `.from_json(text)` | rows, read back in |

A row is an ordinary Lua table, so anything that produces one — `oslo.json.decode`, a registered
tool's handler, a table written by hand — is already input. **None of it touches the shell**, so all
of it works in the three places a pipeline cannot go.

`where` is the one worth knowing about: its expression is Lua, evaluated per row through the engine
that is already running, so calling it from Lua re-enters the VM — and from a registered builtin,
one frame deeper again. That works, and `tests/rows_verb_tests.rs` pins it, because the failure if it
ever regresses is a panic in a prompt rather than a message anyone can read.

### Why a POSIX script cannot reach any of it

Not care: **vocabulary disjointness**. Structure flows only between two stages that both carry a
declaration, and every name that can carry one is a name oslo invented. A script written before oslo
existed cannot name one, so every edge in it plans to bytes. There is no new pipe operator either,
for two reasons: a second operator means the user has to know which to type, and `a |> b` is already
valid POSIX — a redirection of an empty command — so the operator would itself be the hazard.

The argument is worth only as much as its enforcement. With `OSLO_AUDIT_STRUCTURED=1` the shell
registers an `atexit` handler that writes `oslo-audit: structured-edges=<n>` to stderr as the process
ends, and `tests/posix_stays_on_the_byte_path.rs` runs every script in `tests/corpus` — the same
corpus checked against bash byte for byte — and requires that number to be zero every time. A script
that `exec`s is exempt, because it replaces the process image and there is no oslo left to report;
anything else that fails to report is treated as a hole in the measurement, not a pass.

## What makes it different

In bash, zsh and fish a pipe carries bytes and only bytes, so `ls -lh | sort -k5` sorts `4.2G` above
`900M` — text comparison on a rendered size, where `4` sorts before `9` and the gigabyte lands
first. oslo keeps that behaviour for every name those shells
know, and offers the other one only through names they do not have. Adding structure to a POSIX
shell otherwise means either a new operator or a new meaning for `|`; oslo takes neither.

The two renderers are the load-bearing choice. They are two functions rather than one function with
a flag, so the drawn table is not reachable from the code path a pipe takes — a box-drawing
character on another program's stdin is not a bug that can be introduced by forgetting an argument.

A filter is Lua rather than an expression language invented for the occasion, because a filter
language always needs an escape hatch eventually, and then there are two things to learn instead of
one. Here the escape hatch is the filter: `ls | each 'print(name .. " is " .. size)'`.

## Configuration

There is no setting that turns this on or off, and no feature bit for it. The configuration surface
is one function, and it is Lua:

```lua
oslo.register_tool{
  name     = "hosts",
  accepts  = "nothing",              -- nothing | bytes | rows | any
  produces = "rows",                 -- defaults: accepts "nothing", produces "rows"
  rows = function(argv)
    return { { host = "alpha", ip = "10.0.0.1" }, { host = "beta", ip = "192.168.0.2" } }
  end,
}
```

```
$ hosts | where 'ip:match("^10%.")' | cols host ip
host   ip
alpha  10.0.0.1
```

A shape that is not one of the four names is refused by name, because a typo in `produces` would
otherwise make a tool that silently never passes rows on. `oslo.tools()` answers the sorted list of
names a config has registered, which is the only way to tell a tool that failed to register from one
whose name was misspelled.

`run_tool` looks in the config's table before its own, so a name a config registers is the one that
runs.

```sh
OSLO_AUDIT_STRUCTURED=1 oslo script.sh    # stderr: oslo-audit: structured-edges=0
```

## Measurements

Run on this branch, release build, on the machine this was written on.

| what | result |
| --- | --- |
| `tests/posix_stays_on_the_byte_path.rs` over `tests/corpus` | 419 scripts, 0 structured edges, 5.33 s |
| `ls \| grep x` | 0 structured edges |
| `df \| where 'free > 0' \| length` | 2 structured edges |
| `cat pw.txt \| parse '{user}:{x}:{uid}:{rest}' \| where 'uid > 100' \| get user` | 2 structured edges |
| `df` free space, display vs transport | `12G` vs `12534099968` |

The planner's fast path was forced by a measurement recorded in the code: when every stage is a
plain command the terminal is not asked at all, because `is_terminal` is an `ioctl` and it was being
issued once per simple command — including for a bare `x=1` — to decide the last sink of a plan in
which nothing carries rows.

## What it cannot do

* **A redirection in the *middle* of a pipeline takes it back to bytes.**
  `ls | first 2 > mid.txt | cat` answers `first: command not found` and creates an empty file: the
  planner forces text for a redirected stage that is not the last one, because nothing would apply
  its redirection, and with no rows edge left the whole line falls to the byte path. A redirection
  on the *last* stage is fine — `ls | first 2 | to json > o.json` writes the file.
* **Nothing streams.** Every stage materialises the whole table, and the byte prefix is read to end
  of file into a `String` before the first tool runs. `ps | first 1` reads every process; a prefix
  that never ends never returns.
* **Structure cannot cross a process, a function or a compound command**, and a command name that
  comes out of an expansion — `$cmd foo` — is not known when the planner runs, so it is bytes.
* **`oslo.rows` is not a pipeline.** It is the same verbs as functions; it does not make a script's
  `|` carry rows, and it does not give a script the registered tools that produce them.
* **A registered tool only exists at an interactive prompt.** `init.lua` is read by the REPL;
  `oslo -c` and `oslo script.sh` do not read it, so `hosts | where …` in a script is
  `hosts: command not found`.
* `sort-by` is ascending only, with no descending form; `from` knows only `json`; `to` knows
  `json`, `text` and `table`.
* A bare `df`, `ps` or `ls` is the external command, not the structured one — a single stage has no
  edge, so no edge can carry rows. Structure is offered only where it costs nothing.
* The README's `ps | where 'cpu > 10'` cannot work: `ps` rows carry no CPU column, so the filter
  reports `attempt to compare number with nil` and keeps nothing.

## Where it lives

| path | what is in it |
| --- | --- |
| `crates/oslo-shell/src/data/plan.rs` | `Shape`, `Sink`, `Stage`, `plan`, `entered_structured_path` |
| `crates/oslo-shell/src/data/tool.rs` | the declaration registry — `register`, `lookup`, `any_registered` |
| `crates/oslo-shell/src/data/tools/mod.rs` | `register_all` (the whole vocabulary) and `run_tool` |
| `crates/oslo-shell/src/data/tools/verbs.rs` | `cols`, `get`, `sort_by`, `first`, `final_rows`, `length`, `to_format` |
| `crates/oslo-shell/src/data/tools/where_.rs` | `filter` and `for_each` — the Lua binding of a row |
| `crates/oslo-shell/src/data/tools/bridge.rs` | `lines`, `parse`, `from_json` |
| `crates/oslo-shell/src/data/tools/df.rs`, `system.rs` | the `df`, `ps` and `ls` producers |
| `crates/oslo-shell/src/data/value.rs` | `Val`, `Record`, `render_display`, `render_transport` |
| `crates/oslo-shell/src/data/custom.rs` | `register`, `rows_of` — the table a config's tools live in |
| `crates/oslo-shell/src/exec/pipeline/structured.rs` | `structured_sinks`, `run`, `capture` |
| `crates/oslo-shell/src/exec/pipeline/mod.rs` | `run_stages` — the one line where the byte path can be left |
| `crates/oslo-runtime/src/lua/api/tool.rs` | `oslo.register_tool`, `oslo.tools`, and the Record↔Lua converters |
| `crates/oslo-runtime/src/lua/api/rows.rs` | `oslo.rows` — the verbs as functions |
| `src/main.rs` | `register_all` at startup, and `report_structured_audit` |
| `tests/posix_stays_on_the_byte_path.rs` | the corpus assertion |
