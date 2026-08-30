# Structured pipelines

A command in oslo produces two things: text for a person, and rows for the next command. The pipe
decides which one an edge carries **before any stage runs**, by reading what each stage declares —
never by looking at the bytes, because by the time output exists the producer has already chosen a
form and paid to render it.

<!-- demo:begin -->
[![structured-pipelines demo](https://asciinema.org/a/1264169.svg)](https://asciinema.org/a/1264169)
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

### The declaration carries columns, not just shapes

A tool says what shape it takes and gives — and **which columns it will have**. That is the same
decision `plan` already makes about edges, asked one level down, so a column no stage can be carrying
is refused before anything runs:

```sh
$ ls | cols nmae
oslo: cols: nmae: no such column          # and `ls` never ran
```

About twenty-five of the forty verbs answer exactly. The three producers declare their columns beside
the code that fills them, and a test runs each one to check the declaration has not drifted from the
rows. `parse` is the surprise: its columns are sitting in a literal operand, so
`parse '{user}:{uid}'` is knowable before a byte of input arrives. The rest — `from json`, `map`,
`flatten`, `headers`, `lookup` — are **`Unknown`**, and *nothing may be refused on an `Unknown`*: a
plan-time check that guesses wrong turns a working pipeline into an error, which is worse than the
runtime check it replaces. `tools::unknown_column` still catches everything the planner cannot see.

**Quoting is not expansion.** `where 'size > 100'` is as knowable as `where true` — it is the same
text however the shell is feeling — so a quoted stage is read like any other and the columns after it
stay `Known`. A word that really does depend on the environment, `cols $c`, is where the planner
stops and the rows take over.

The same knowledge answers the question a person has at the prompt:

```
ls | sort-by <Tab>      name  size  size_human  is_dir  modified  mode
ls | reject size | sort-by <Tab>    name  size_human  is_dir  modified  mode
```

And inside a filter, where a column name is most often typed:

```
ls | where 'siz<Tab>          size  size_human
ls | where 'size > 1 and na<Tab>   name          — only the identifier is replaced
```

`where`, `map`, `each`, `reduce` and the three that compute all bind the row's columns as globals, so
the names are as nameable there as in an operand. A name after a `.` or a `:` is **not** offered:
`row.na` is a field and `name:up` is a method, and splicing a column into either would mean something
else entirely.

The offer follows the pipeline, because it is the same algebra: a column a verb made is offered, one
it removed is not. A column position whose columns are unknowable offers **nothing** rather than
falling through to filenames — a filename where a column belongs is the wrong nothing.

The vocabulary is the whole of what can carry structure:

| name | accepts | produces |
| --- | --- | --- |
| `df` `ps` `ls` | nothing | rows |
| `lines` `parse` `from` `detect-columns` | bytes | rows |
| `where` `map` `each` `cols` `get` `sort-by` `reverse` `first` `final` `length` | rows | rows |
| `group-by` `count` `distinct` `stats` `describe` `histogram` `reduce` | rows | rows |
| `lookup` `append` `merge` | rows | rows |
| `reject` `rename` `insert` `update` `upsert` `flatten` `headers` | rows | rows |
| `skip` `every` `enumerate` `compact` `default` | rows | rows |
| `to` | rows | bytes |

`cols` rather than `select`, because `select` is a bash keyword and oslo's parser refuses it as one.
`from json` rather than `from-json`, because the format is an argument and a format oslo learns
later then needs no new command name.

`map` and `each` are two names for two things: `map` answers a row per row, `each` answers none and
the pipeline ends there. A flag on one would make "does this produce rows" a runtime question, and
the planner has to know it before anything runs.

**The vocabulary is rationed on purpose.** Every name registered is a name a POSIX script might
already call, so the list above is not "what would be useful" but "what has no expression in the
rest of it". `take` is not here because `first` is; descending order is a flag on `sort-by` rather
than a verb of its own. See `data/tools/reshape.rs` for the ten that were considered and refused.

`lookup` rather than `join`, and that one is not a preference: **`join` is POSIX.1** and coreutils
ships it. A rows producer piped into a name a script already calls is exactly the defect `uniq` had.

### A stream, when every part of one can be

`tail -f app.log | lines | where 'line:match("ERROR")'` printed nothing at all, for ever: the byte
prefix was read to end of file before the first verb ran, and a follow has no end. Now the upstream
is read in slices, each slice becomes rows, those go through the verbs, and what comes out is
printed — then it goes back for more.

```sh
tail -f app.log | lines | where 'line:match("ERROR")'   # prints as the log grows
tail -f app.log | lines | each 'print(line)'            # side effects, as they happen
yes | lines | first 2                                   # answers instantly, in 21 MB
seq 1 40000000 | lines | length                         # 40M rows counted in 22 MB
```

That last one used to **fail**: a ~350 MB upstream hit the 256 MiB cap, for a question whose answer
is one integer. `length` and `final n` answer only once the stream ends, but what they hold while
they wait is bounded — a counter, and n rows — so the upstream's size stops being the limit.

When a verb has had enough the reader is **closed**, which is what gives the upstream its `SIGPIPE`
and ends it — the mechanism `yes | head -2` has always used, arriving in the structured half at last.

A pipeline is streamed only when every part of it can be; otherwise nothing changes:

| streams | does not |
|---|---|
| a plain external upstream | a builtin, a function, a compound, anything redirected |
| `lines`, `parse` — a row per line; `from csv`, `from tsv` — a row per *record* | `from json` needs the closing brace, `detect-columns` needs every row to find the columns |
| `where` `map` `cols` `get` `reject` `rename` `flatten` `compact` `default` `upsert` `each` | `sort-by` `group-by` `stats` `reverse` — each has to hold the whole stream to answer |
| `first` `skip` `every` `enumerate`, counting across slices | `from json`, `detect-columns`, `lookup` `append` `merge` |
| `length` `final n`, folding into a bound and answering at the end | `insert` and `update` **where the columns are not known** |

`insert` and `update` refuse on a question only the whole stream answers — whether a column exists
anywhere in it — so applied per batch they could refuse the third batch after emitting the first
two. What makes them safe is the column contract: where the set is `Known`, that question was
already settled before anything ran, so no batch can disagree. Where it is `Unknown` the pipeline
materialises, as it did before. `upsert` needs no such gate, because refusing nothing is the whole
of what makes it `upsert`.

A delimited document is the awkward one, and both of its awkwardnesses are about where a batch may
end. A quoted field may contain a newline, so a batch ends at the last newline that leaves every
field closed — asked of the real parser rather than of a copy of its rules. And the first record
names the columns, so it is remembered and put back in front of every batch after it, which is what
lets the same parser answer for a slice as for the whole document. `from json` has neither problem
and cannot stream at all: it has nothing to say until the closing brace.

The upstream runs through the ordinary byte path inside a forked child rather than being spawned
some other way, so argv, `$PATH`, the environment and the exit status are the byte path's by
construction and cannot drift from it.

**A stream cannot be a table.** Aligning columns needs the widest value, which needs the last row. So
streamed output is a header and then one line per row, each cell rendered for a person but not
padded. Holding every row in order to align it is the thing this exists not to do.

### A second stream

`lookup`, `append` and `merge` need a stream that the pipeline cannot give them — it is a line, and
there is no `|` shape for "and also read this". They name the other side as a **Lua expression**,
evaluated once:

```sh
ls | lookup 'sh.stat("Cargo.toml", "README.md")' name
ls | lookup --keep '{ {name="README.md", kind="docs"} }' name
ps | append 'oslo.rows.from_json(saved)'
```

`lookup` is an inner join by default — a left row with no match does not survive, so "did this
match?" stays answerable — and `--keep` is the left-outer form. A column both sides have arrives as
`<name>_2`, because overwriting loses data silently and skipping loses it loudly. `merge` pairs by
position and `append` concatenates.

The prettier shape is `lookup (ls) name`, with the operand evaluated as a structured pipeline of its
own. That needs the planner to recurse into an operand, which it cannot do today — so the Lua form
is what exists, and it stays as the escape hatch when the other arrives.

The producers' columns, as they actually come out:

| producer | columns |
| --- | --- |
| `df` | `filesystem` `size` `used` `free` `capacity` `mounted` |
| `ps` | `pid` `name` `cmdline` `is_kernel` |
| `ls` | `name` `size` `size_human` `is_dir` `modified` `mode` |

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

### What a cell is, once it reaches Lua

A cell arrives as the nearest thing Lua actually has, and **never as its rendering** — that is what
makes `free < 1e9` arithmetic rather than a comparison of the characters `4.2G`.

| cell | in Lua |
| --- | --- |
| a size | an integer, its number of bytes |
| a duration, a time | an integer, its number of nanoseconds |
| bytes that are not text | a Lua byte string, unchanged |
| a failed cell | `{ error = "…" }` |

A failed cell is a table rather than `nil` or a string because it has to be distinguishable from
both: `nil` is already what an absent column is, and a string is already what a column of text is,
so under either a filter could not tell a cell that *failed* from one that legitimately held that
value. It is the shape `to json` gives the same cell, and a filter that wants to find one asks
`where 'type(free) == "table" and free.error ~= nil'` — the type test first, because indexing the
number that a good row holds there would raise.

There is **one** converter each way, in `data/lua.rs`, and that is load-bearing rather than tidy:
`where` binds a row's columns through it, `oslo.register_tool` hands a tool its input through it, and
`ps` and `ls` read their own rows back through it. When those were three separate copies they
disagreed — the same blob was its length to a filter and a lossy string to the tool the filter fed.

### Units in a filter

`where 'size > 1GB'` reads the way it is written. A numeral followed by a unit is not Lua and cannot
be made into Lua, so the literal is replaced by the number the rows carry *before* the expression is
compiled — a size becomes bytes and a duration becomes nanoseconds, which is exactly what the cell
already holds.

```sh
ls | where 'size > 1MB'
df | where 'free > 1GiB' | cols mounted free
ps | where 'cpu_time > 5min'
```

Sizes are `kB` `MB` `GB` `TB` and the binary `KiB` `MiB` `GiB`; durations are `ms` `s` `min` `h` `d`.
One space is allowed, so `1 GB` reads too. **The SI prefix is case-sensitive** — `1kB` is a kilobyte
and `1KB` is not a unit at all, so it stays as it was and Lua reports it as the syntax error it is.
That is the general rule: a literal the calculator does not know is left exactly as written, so an
expression this does not understand fails the way it always did rather than in some new way.

The scan is deliberately narrow, and leaves `1e3` (already a number), `0x1f` (a hex numeral), `x1GB`
(a name), `'1GB'` (inside quotes) and `n > 1 and m` (`and` is a keyword, not a unit) alone.

This is the one part of the vocabulary behind a feature: it asks the calculator what a unit is worth,
so it needs `math`. Release binaries are built with every feature; a plain `cargo build` is not, and
there a unit literal stays text.

### The verbs that make a stream smaller

Every other verb is *selection* — it keeps rows and throws rows away, and none of it can answer *how
many*, *which distinct*, or *how much in total*.

```sh
ps | group-by is_kernel | count
ls | distinct is_dir
df | stats free
```

| verb | answers |
| --- | --- |
| `group-by C` | one row per distinct value of `C`, carrying `count` and the `rows` themselves |
| `count` | how many rows — or, after `group-by`, how many in each group |
| `distinct [C]` | the first of each distinct row, or of each distinct value of `C` |
| `stats C` | one row: `field` `count` `min` `max` `sum` `mean` over the numbers in `C` |

`group-by` keeps the rows it grouped, so it composes with everything after it rather than being a
dead end only `count` can follow; its order is first-seen, because a pipeline that wants order says
`sort-by` and a `group-by` that sorted would quietly undo one. `distinct` rather than `uniq`, and
`final` rather than `last`, for the reason the whole vocabulary is oslo's own: `uniq` and `last` are
commands people already have, and taking those names is how `ls | uniq` stops meaning what it said.

**`join` is not here.** It needs a *second* input stream, and the pipeline is a line — there is no
shape for "and also read this". Adding one is a change to the pipeline, not another verb.

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

**A tool is not only a source.** The handler is `function(argv, input, bytes)`, and what it is given
is what it declared: `input` is the rows of the stage before it when `accepts` is `"rows"` or
`"any"`, and `bytes` is that stage's output as one string when it is `"bytes"`. Lua ignores
arguments a function did not declare, so `function(argv)` keeps working unchanged.

```lua
oslo.register_tool{
  name    = "redact",
  accepts = "rows",
  rows = function(argv, input)
    for _, row in ipairs(input) do row.ip = "x.x.x.x" end
    return input
  end,
}
```

Either argument is `nil` rather than empty when the tool never asked for it: "I was given nothing"
and "I was given no rows" are different questions, and a verb that filters wants to tell them apart.
Declaring `"bytes"` copies the whole stream into a Lua string, so a tool facing a 200 MB pipe costs
200 MB — one that wants to stream should take rows from `lines` in front of it instead.

**A tool may say what its rows have**, in the same table:

```lua
oslo.register_tool{
  name    = "hosts",
  columns = { "host", "ip" },
  rows    = function() return { { host = "alpha", ip = "10.0.0.1" } } end,
}
```

That buys the two things the built-in producers get: `hosts | cols hsot` is refused **before the
tool runs**, and Tab offers `host` and `ip`. It matters most here — a config's tool is the one that
might *do* something on the way to producing rows. A tool that says nothing is `Unknown`, and
nothing is ever refused on an `Unknown`, so every tool written before this behaves as it did.

A shape that is not one of the four names is refused by name, because a typo in `produces` would
otherwise make a tool that silently never passes rows on. `oslo.tools()` answers the sorted list of
names a config has registered, which is the only way to tell a tool that failed to register from one
whose name was misspelled.

`run_tool` looks in the config's table before its own, so a name a config registers is the one that
runs.

Unlike `df`, `ps` and `ls`, a tool a config registered **runs on its own**, with no pipeline around
it. Those three have an external command of the same name to fall back to and a lone `ls` must stay
coreutils; a name a config invented has no such counterpart, so `hosts` answering `command not found`
until it was piped somewhere would not be a discoverable interface for a feature whose whole point is
adding a command.

### The drawn table

`oslo.table` configures the face a person reads, and **only** that face:

```lua
oslo.table.index      = true   -- a leading column of row numbers
oslo.table.max_column = 60     -- the widest one cell may be drawn; 0 is no limit
oslo.table.null       = "-"    -- what an absent or null cell shows
```

None of it can reach `render_transport`. That is what the two renderers being two functions is for:
a setting that changed the transport would put somebody's preference on another program's standard
input.

```sh
OSLO_AUDIT_STRUCTURED=1 oslo script.sh    # stderr: oslo-audit: structured-edges=0
```

## Measurements

Run on this branch, release build, on the machine this was written on.

| what | result |
| --- | --- |
| `tests/posix_stays_on_the_byte_path.rs` over `tests/corpus` | 432 scripts, 0 structured edges, 3.03 s |
| `ls \| grep x` | 0 structured edges |
| `df \| where 'free > 0' \| length` | 2 structured edges |
| `cat pw.txt \| parse '{user}:{x}:{uid}:{rest}' \| where 'uid > 100' \| get user` | 2 structured edges |
| `ps \| group-by is_kernel \| count` | 2 structured edges |
| `df` free space, display vs transport | `12G` vs `12534099968` |

The planner's fast path was forced by a measurement recorded in the code: when every stage is a
plain command the terminal is not asked at all, because `is_terminal` is an `ioctl` and it was being
issued once per simple command — including for a bare `x=1` — to decide the last sink of a plan in
which nothing carries rows.

## What it cannot do

* **A verb in the middle of a pipeline cannot redirect its *output*.** Rows cross in memory rather
  than on a descriptor, so a verb whose stdout went to a file would leave the next stage nothing to
  read. `ls | first 2 > mid.txt | cat` is refused by name, with status 2 and no file created. A
  redirection on the *last* stage is the ordinary case — `ls | first 2 | to json > o.json` writes
  it — and a redirection that leaves stdout alone, `2>/dev/null` among them, is applied around its
  own stage and changes nothing about the rows.
* **Most of it materialises.** A pipeline that cannot be streamed builds every table whole and reads
  its upstream to the end first — `ps | first 1` reads every process, which costs nothing
  measurable. An upstream with no *end* is refused at 256 MiB rather than swallowed:
  `yes | lines | sort-by line` cannot stream. Not because `sort-by` needs the last row — `length`
  needs it too and streams — but because it has to **hold every row** to answer, and that is the
  cost the cap exists to bound.
* **Structure cannot cross a process, a function or a compound command**, and a command name that
  comes out of an expansion — `$cmd foo` — is not known when the planner runs, so it is bytes.
* **An alias of your own is outranked by a verb inside a pipeline, and only there.** The vocabulary
  is disjoint from POSIX and coreutils, not from names you have already taken — `alias df=dfc` and
  `alias get='sudo sysget'` are ordinary. Aliases expand before the pipeline is planned, so those
  used to make `df | where …` and `ls | get name` impossible.

  Position decides it now, and the two readings never overlap. A bare `df` is the alias, as a bare
  `df` has always been the external command. `df | where …` is the verb, because another verb in the
  line says what it is for. **A pipe alone is not enough**: `ls | cat` keeps whatever `alias ls`
  says, since nothing there asks for rows. Quoting still forces the verb — `\ls`, `'ls'` — and a
  verb reported missing lists the aliases carrying verb names.
* **`oslo.rows` is not a pipeline.** It is the same verbs as functions — every one that transforms
  rows, which a test asserts against the registry so the two cannot drift apart. What it is not is a
  pipeline: it does not make a script's `|` carry rows, and it does not give a script the registered
  tools that produce them. The producers and the bridges are excluded, each for its own reason:
  `ls`, `ps` and `df` read the machine rather than rows; `lines`, `parse`, `from json`, `from csv`
  and `from tsv` take *text*, and are bound under those names; `to` is `render`.
* **A cell may name its own kind.** A size, a duration and a time reach Lua as plain numbers, so
  that `free < 1e9` is arithmetic rather than a string comparison — which means a number handed back
  cannot say which of them it was, and every Lua-valued verb used to flatten it: `ls | … | map "{
  size = size }"` drew `53724` where the cell it came from drew `52K`. `oslo.rows.size(n)`,
  `duration(ns)`, `time(ns)` and `fail(message)` write the four kinds Lua could not otherwise make,
  so a tool a config registers can answer with rows that draw exactly as `ls` and `df` do:

  ```lua
  oslo.register_tool{ name = "stale", produces = "rows", rows = function()
    return { { name = "nixpkgs", age = oslo.rows.duration(400 * 86400 * 1e9) } }
  end }
  ```
* **A registered tool reaches a script only if the script asks for it.** `init.lua` is read by the
  REPL and by nothing else — deliberately, because on a machine where oslo is `/bin/sh` every
  `sh -c` in every Makefile would otherwise run the person-at-the-keyboard's config. A script says
  what it wants instead, and `source` detects that the file is Lua:

  ```sh
  source ~/.config/oslo/tools.lua
  hosts | where 'ip:match("^10%.")'
  ```

  A sourced Lua file can **register** — a tool lands in the same thread-local table `init.lua` puts
  one in. It cannot set shell variables: Lua reached from inside a builtin runs while the shell
  holds its own state, which is the constraint `env::view` exists for, not a shortcut taken here.
* `from` knows `json`, `csv` and `tsv`; `to` knows `json`, `csv`, `tsv`, `text` and `table`. The
  three verbs that need a *second* input — `lookup`, `append`, `merge` — take theirs as a Lua
  expression rather than a second pipeline, because the pipeline is a line and has no shape for
  "and also read this". See **A second stream**.
* A bare `df`, `ps` or `ls` is the external command, not the structured one — a single stage has no
  edge, so no edge can carry rows. Structure is offered only where it costs nothing. A tool a config
  registered is the exception, and runs on its own; see **Configuration**.
* **`detect-columns` guesses, and can be wrong.** Two columns the header packs one space apart,
  whose values also touch on some row, stay merged — `ps aux` does it with `RSS TTY` on a busy
  machine. A column empty on every row is invisible. Both want `parse` with a pattern, which is why
  that verb is not going anywhere.
* **A unit literal needs the `math` feature**, since it asks the calculator what a unit is worth.
  Release binaries have every feature; a plain `cargo build` does not, and there `where 'size > 1GB'`
  is the Lua syntax error it always was.

## The invariants

Six things hold however the structured half grows. The first two are asserted by tests rather than
trusted, because they are the POSIX guarantee and a guarantee nobody checks is a hope.

| # | invariant | enforced by |
|---|---|---|
| **I1** | A POSIX script never takes a structured edge | `tests/posix_stays_on_the_byte_path.rs` |
| **I2** | Every structured name is one oslo invented | `tests/structured_names_are_oslos_own.rs` |
| **I3** | The drawn table can never reach a pipe | two renderers, no shared flag |
| **I4** | No new pipe operator | design |
| **I5** | A tool's declaration decides the edge, never its bytes | `plan()` reads `Shape` only |
| **I6** | Statuses describe the pipeline as written | `pipeline_status`, shared |

## What is permanently out

Not "not yet". These were each considered against a specific alternative and rejected, and the
reasons do not expire — a later version wanting one of them is a later version disagreeing with the
design rather than extending it.

| idea | why not |
|---|---|
| An out-of-process plugin protocol | nushell needs one because its extension language is itself. oslo's is Lua, in process, and `register_tool` already covers it. A subprocess protocol on a static musl binary buys nothing and costs a wire format |
| Static parse-before-evaluate | nushell's foundation, and flatly incompatible with being a POSIX shell — no dynamic `source`, no `eval` |
| `par-each` | impossible, not merely hard: the Lua VM is `Rc<RefCell<…>>` and not `Send` |
| A separate `table` type | "a table is a list of records" is deliberately one shape fewer to think about at every step |
| `open` `save` `sort` `find` `tee` `math` `join` | names POSIX and coreutils already have. This is **I2**, and `uniq` already proved what ignoring it costs |
| yaml, toml, ini, xml | each costs a dependency, and the release is a static musl binary with no C toolchain. csv and tsv are hand-rolled for exactly this reason |

## Where it lives

| path | what is in it |
| --- | --- |
| `crates/oslo-shell/src/data/plan.rs` | `Shape`, `Sink`, `Stage`, `plan`, `entered_structured_path` |
| `crates/oslo-shell/src/data/tool.rs` | the declaration registry — `register`, `lookup`, `any_registered` |
| `crates/oslo-shell/src/data/tools/mod.rs` | `register_all` (the whole vocabulary) and `run_tool` |
| `crates/oslo-shell/src/data/tools/verbs.rs` | `cols`, `get`, `sort_by`, `first`, `final_rows`, `length`, `to_format` |
| `crates/oslo-shell/src/data/tools/summarise.rs` | `group_by`, `count`, `distinct`, `stats` |
| `crates/oslo-shell/src/data/tools/reshape.rs` | the twelve reshaping verbs, and the ten refused |
| `crates/oslo-shell/src/data/tools/second.rs` | `lookup`, `append`, `merge` — the verbs that need a second stream |
| `crates/oslo-shell/src/data/tools/detect.rs` | `detect-columns` — three rules for finding columns in somebody else's output |
| `crates/oslo-shell/src/data/tools/formats.rs` | `from csv`, `to csv` and their tab-separated twins |
| `crates/oslo-shell/src/data/path.rs` | `Path` — `metadata.name`, `images.0`, and the `?` that allows a gap |
| `crates/oslo-shell/src/data/columns.rs` | `Columns`, `through` — the algebra, and what each verb does to a column set |
| `crates/oslo-shell/src/data/complete.rs` | `columns_at` — what may be named at a point in a half-typed line |
| `tests/column_contract_tests.rs`, `tests/column_completion_tests.rs` | the two behaviours the contract buys |
| `crates/oslo-shell/src/data/tools/where_.rs` | `filter` and `for_each` — the Lua binding of a row |
| `crates/oslo-shell/src/data/tools/units.rs` | `expand` — `1GB` becomes its number before the filter compiles |
| `crates/oslo-shell/src/data/tools/bridge.rs` | `lines`, `parse`, `from_json` |
| `crates/oslo-shell/src/data/tools/df.rs`, `system.rs` | the `df`, `ps` and `ls` producers |
| `crates/oslo-shell/src/data/value.rs` | `Val`, `Record`, `render_display`, `render_transport` |
| `crates/oslo-shell/src/data/lua.rs` | `to_lua`, `from_lua`, `rows_value`, `records_of` — the one crossing between a cell and a Lua value, both ways |
| `crates/oslo-shell/src/data/custom.rs` | `register`, `rows_of` — the table a config's tools live in |
| `crates/oslo-shell/src/exec/pipeline/structured.rs` | `structured_sinks`, `run`, `capture` |
| `crates/oslo-shell/src/exec/pipeline/structured/stream.rs` | reading an upstream in slices when every part of a pipeline can be streamed |
| `crates/oslo-shell/src/exec/pipeline/structured/handover.rs` | `byte_suffix_at`, `hand_over`, `Printed` — where the tools stop and bytes take over |
| `crates/oslo-shell/src/exec/pipeline/mod.rs` | `run_stages` — the one line where the byte path can be left |
| `crates/oslo-runtime/src/lua/api/tool.rs` | `oslo.register_tool` and `oslo.tools` |
| `crates/oslo-runtime/src/lua/api/rows.rs` | `oslo.rows` — the verbs as functions |
| `src/main.rs` | `register_all` at startup, and `report_structured_audit` |
| `tests/posix_stays_on_the_byte_path.rs` | the corpus assertion |
