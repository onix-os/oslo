# The dual-channel pipe

A design for oslo's structured pipeline, drawn from a study of nushell's data model and pipeline
internals, Hilbish's sinks, and the constraint that oslo intends to be `/bin/sh`.

The shape in one sentence: a command emits human text *and* machine-readable rows, and the pipe
decides which channel an edge carries **before anything runs**, from declarations, never by
inspecting bytes.

## The rule that outranks the rest

**fd 1 never carries structure, under any option, in any mode.** Display rendering and transport rendering are two different functions and must be written as two functions from the first commit.

Nushell's most damaging bug is that piping a table to an external program renders box-drawing
characters onto its stdin. If a rendered cell ever reaches a pipe in oslo, the premise of the
project is gone.

## The value model

A NEW Rust enum in `src/data/`, NOT `lua::eval::value::Value`. Three reasons the Lua type cannot
do this job. (a) It is `Rc<RefCell<Table>>` — `!Send`, and only valid while an interpreter is
live on this thread; a pipeline value has to survive a fork and be constructible by a Rust
builtin with no Lua in sight. (b) `Table`'s hash part is a `HashMap`
(src/lua/eval/value.rs:150), so column order is not preserved — `df` would print its columns in
a different order every run. (c) It has no room for tagged scalars, and cramming
`{__kind="size"}` markers into tables makes every consumer defensive.

```rust
// src/data/value.rs
pub enum Val {
    Null, Bool(bool), Int(i64), Float(f64),
    Str(Box<str>),
    Bytes(Vec<u8>),        // binary, distinct from Str — without this `open` on a JPEG is mojibake
    Size(u64),             // byte count; renders "4.2G", compares as a number
    Duration(i64),         // nanoseconds
    Time(i64),             // unix nanoseconds
    List(Vec<Val>),
    Record(Record),
    Error(Box<DataError>), // a per-CELL failure that does not abort the stream
}

pub struct Record { cols: Vec<Box<str>>, vals: Vec<Val> }  // insertion-ordered, Vec-backed
```

`Record` is two parallel `Vec`s with linear lookup, not a map: records are 3-15 columns wide, a
hash is slower at that size and loses the order that decides how columns are drawn and
serialised. A *table* is `List(Vec<Val::Record>)` — there is no fourth type. Nushell got that
right and it means oslo needs no new concept: it is already what `sh.df()` returns.

`Val::Error` as a variant is load-bearing and easy to skip. `ps` on a live box hits a process
that exits between readdir and read; `df` hits a stale NFS mount. Text tools warn on stderr and
continue, and that is *why* people trust them. If the structured path aborts the listing
instead, the structured path becomes the untrustworthy one and nobody uses it. Put it in the
value model on day one or every tool's inner loop changes later.

The tagged scalars are what make the query surface worth having: `Size` is what lets `where
'free < 1gb'` mean something. `docs/built-in-tools.md` already committed to `size` +
`size_human` as two fields; `Val::Size` is that decision done once in the renderer instead of
once per tool, and it removes the failure mode the doc itself names (the two fields
disagreeing).

STREAMING — decided now, not deferred as the doc suggests. The pipe carries `Data`, not `Val`:

```rust
pub enum Data {
    Value(Val),        // bounded and in hand: df, env, stat
    Rows(RowStream),   // arriving over time: find /, a tailed log
    None,              // no structure on this edge
}
pub struct RowStream {
    next: Box<dyn FnMut() -> Option<Val>>,
    interrupt: Arc<AtomicBool>,      // IN the stream, from day one
    cols: Option<Vec<Box<str>>>,     // header hint so the renderer can draw before row 1
}
```

The interrupt flag goes in the constructor now because nushell's decade of "this particular loop
forgot to check ctrl-C" (their issues #7477, #12028, #12649) is the direct consequence of adding
it late. oslo's existing SIGINT machinery (src/exec/pipeline/interrupt.rs) unwinds the
*evaluator*; it has no answer for one builtin iterating ten million rows in-process, which is
exactly what this feature introduces. One field now, twenty patches later.

Collect points are a declared property, not an emergent one: a tool declares `collects: bool`
(`sort-by`, `reverse`, `length` do; `where`, `each`, `first`, `get` do not) and `oslo` can print
where a given pipeline materialises. Nushell's worst unforced error is that streaming was
retrofitted, so collect points crept in one command at a time and nobody could see them — `to
csv | save` still buffers, open since Nov 2024. A shell that is /bin/sh on a distro will be
handed a log file bigger than RAM.

## What the pipe does

The pipe does NOT sniff. It cannot: by the time output exists the producer has already chosen a
form and paid to render it. The destination is decided BEFORE the producer runs, by walking the
pipeline right-to-left — nushell's `OutDest`, which is the one thing they got structurally
right.

```rust
enum Sink { Print, Text, Rows }
fn plan(stages: &[Stage]) -> Vec<Sink>
```
- Last stage → `Print` if fd 1 is untouched by redirection, else `Text`.
- Edge i→i+1 → `Rows` iff stage i declares `produces: Rows` AND stage i+1 declares `accepts: Rows` AND there is no redirection on either side of that edge AND both are in-process (builtin/tool, not external, not a function, not a compound). Otherwise `Text`.

`produces`/`accepts` are STATIC properties of the registered tool, read off its signature —
never guessed from bytes. This is the cheap, predictable answer to "does the next stage
understand structure?", and it is the half of the owner's sentence that has to be inverted: the
pipe does not figure out that the previous command output two things, the pipe TELLS the
previous command which one to produce.

builtin | builtin, both structured — `df | where 'free < 1gb' | sort-by free | first 5` Sink is
`Rows`. And here is the change to `run_stages`: today every stage of an N>1 pipeline is forked
(src/exec/pipeline/mod.rs:350-394). A contiguous run of structured stages with no redirections
**does not fork at all** — it collapses into one process and chains as iterators. Zero pipes,
zero serialisation, zero copies, and streaming falls out of the iterator. The producer's text
renderer is never called, so `df` never pays to format columns nobody reads.

builtin | external — `df | awk`, `ls | grep foo` Sink is `Text`. The producer writes its text
face to fd 1 and the current code path runs unmodified. `df` prints what `df` prints. **This is
the default**: structure is used only when positively proven usable at both ends.

Diverging from nushell explicitly here, because this is their single most damaging bug: piping
structured data to an external in nu renders the box-drawing table (`╭─┬─╮`) onto the external's
stdin, which is why their docs tell users to insert `to text` by hand. It is impossible in
oslo's design only if the rule is stated as an invariant: **the display rendering and the
transport rendering are two different functions.** `Sink::Print` may use colour, borders, width-
fitting, `size_human`. `Sink::Text` is plain, uncoloured, unfitted, untruncated, un-abbreviated,
one record per line. A tool with only one renderer uses the transport one for both. If a `│ 4.2G
│` ever reaches a pipe the project's premise is dead.

external | builtin — `kubectl get pods -o json | from json | where 'phase == "Running"'` The
external's stdout is bytes. The consumer declares `accepts: Bytes` and MANUFACTURES the
structure. This is the adapter layer — `from json`, `lines`, `parse "{a} {b}"`, `detect columns`
— and it needs no cooperation from the external at all. This is where nearly all the day-one
value is, because it works with kubectl, docker, gh, jq, systemctl, ip, lsblk and cargo on the
machine the owner is running today, rather than after the distro's tools are written. A dual-
channel pipe where only oslo's five tools have a channel is a structured world five commands
wide.

external | external — `ls | grep foo` Nothing whatsoever. `plan()` returns `Text` everywhere,
`run_stages` takes the existing branch, no extra fd is opened, no variable is exported, nothing
between `fork` and `exec` differs by one instruction.

Structure is DROPPED — silently and always falling back to bytes — when: any redirection touches
the connecting descriptor; either end is an external, a function, a compound command, or a
builtin without the declaration; the stage is backgrounded; the pipeline is inside `$(...)`
(command substitution captures bytes, so the last stage is `Text`, never `Print`); or the
consumer's `accepts` does not include `Rows`. `time` and `!` are transparent — they touch status
and timing, not the data path.

PIPESTATUS is the one thing the collapse must not break. `set_pipeline_status`
(src/env/scope.rs:224) needs one status per *stage*, and a collapsed run has one process.
Synthesise it: each collapsed stage records its own status as it finishes, and the vector handed
to `set_pipeline_status` has the same length it would have had. `pipefail` reads the same
vector, so both stay consistent.

## Why POSIX is safe

Four independent arguments, the last of which is mechanically checkable rather than a promise.

(1) VOCABULARY DISJOINTNESS. Structure flows only between two commands *both* carrying a
declaration. Every name that can carry one is either invented by oslo (`where`, `get`, `cols`,
`from`, `to`, `each`, `sort-by`, `first`) or is an existing builtin deliberately declared
`produces: Bytes` (`echo`, `printf`, `cat`, `read`, `test`). A script written before oslo
existed cannot name a structured consumer. Therefore `plan()` returns `Text` for every edge in
it. Therefore it takes the byte path. **The set of scripts whose behaviour can change is exactly
the set of scripts that mention a name oslo invented** — which is empty for every script on the
machine.

This is also why there is NO new pipe operator. No `|>`, no `||>`. Two reasons: a second
operator means the user must know which to type, which violates "nobody should have to learn a
new language to use the shell"; and `a |> b` is currently valid POSIX (`a | > b`, a redirection
of an empty command), so the operator is itself a compat hazard. The parser is not touched, the
AST is not touched, the vendored brush fork is not touched. Plain `|` does the right thing
because both ends are declared.

(2) `Text` IS THE CURRENT CODE PATH, NOT AN EQUIVALENT ONE. `run_stages` gets exactly one new
early branch:
```rust
let sinks = plan(&pipeline.commands);
if !sinks.iter().any(|s| *s == Sink::Rows) {
    return run_stages_posix(env, pipeline);   // today's function, verbatim, unedited
}
```
`run_stages_posix` is the existing body moved, not rewritten. There is no path by which the
structured machinery can affect a pipeline that has no structured stage, because the structured
machinery is never entered.

(3) NO OBSERVABLE ENVIRONMENT CHANGE. Nothing is exported on the external path. When cross-
process transport eventually lands, `OSLO_DATA_FD` is set only when the planner chose `Rows` for
that specific edge, which for an external requires it to have been declared structure-capable by
config. `ls | grep foo` exports nothing, opens no descriptor, and dup2s nothing: `env` inside
`grep` is byte-identical to what bash hands it. A program that inspects its own environment or
its own open descriptors cannot tell oslo from dash.

(4) FD 1 IS NEVER USED FOR STRUCTURE. Under any option, in any mode, at any verbosity. Structure
is an in-memory iterator or a different descriptor, full stop. This is the single invariant
that, if broken once, ends the project — worth writing into the design doc as a rule rather than
leaving it to be rediscovered.

ENFORCEMENT, not argument. oslo already differential-tests 375 scripts against bash. Land this
with (a) the corpus green, and (b) `run_stages` instrumented so the structured path increments a
counter, and a test asserting the counter is **zero** after the whole corpus runs. That converts
"POSIX is safe" from a claim into a build failure when it stops being true. Then add a second
corpus of deliberately mixed pipelines — structured producer into `awk`, `grep` into structured
consumer, structured stage with a redirection, structured stage backgrounded, structured
pipeline inside `$( )` — each asserted byte-identical to what the text face alone produces.

One honest disclosure. The collapse in case (1) of the pipe behaviour means two structured
stages share a process, where bash would fork. That IS an observable difference (a variable
assigned in the right-hand stage would survive). It cannot affect any existing script, because
it is gated on both stages being names oslo invented. It should still be documented as a
deliberate divergence rather than discovered, and the gate should be asserted in code: a stage
that is a function, a compound, an external, or any builtin without `produces: Rows` forks
exactly as it does today.

## The Lua surface

`oslo.register_builtin` today hands the callback only an argv table and takes back an exit
status (src/lua/engine.rs:71-95, src/lua/api/mod.rs:292). The callback has no handle on its own
stdout — Lua `print` writes to process stdout and lands in the pipe only by accident of the fork
in `run_stages`. A dual-channel pipe cannot be built on that, because the structured channel is
by definition not fd 1. **Widen the signature before it has users**; changing it after configs
exist is a break.

EMITTING. The recommended form separates facts from rendering, which is what `docs/built-in-
tools.md` already argues for and what stops the two faces drifting:

```lua
oslo.register_tool{
  name     = "hosts",
  accepts  = "nothing",          -- "nothing" | "bytes" | "rows" | "any"
  produces = "rows",
  cols     = {"host", "ip", "seen"},
  rows   = function(argv)  return { {host="a", ip="10.0.0.1", seen=oslo.time.now()}, ... }  end,
  render = function(rows, out)  for _, r in ipairs(rows) do out:write(r.host.."\t"..r.ip.."\n") end  end,
}
```
`render` is optional — omit it and the default transport renderer is used. When the planner
chose `Rows`, `render` is **never called**, so the tool does not pay for a rendering nobody
reads. One source of facts, one renderer, no possibility of the two disagreeing.

`run` is the escape hatch for tools whose text face is genuinely not a rendering of the rows
(`ls -l --color`, or anything with no row shape at all). It gets the stream object — Hilbish's
`sinks`, with the second channel present from the start:

```lua
run = function(argv, io)
    for line in io.input:lines() do ... end   -- nil when accepts == "nothing"
    for row  in io.input:rows()  do ... end   -- non-nil only when the previous stage was structured
    io.out:write("text a human reads\n")      -- fd 1, real descriptor
    io.err:write("diagnostic\n")              -- fd 2
    io.rows:emit{ host = "a", ip = "10.0.0.1" }   -- the structured channel
    return 0
end
```
`io.sink` is `"print"`, `"text"` or `"rows"` — nushell's `is_redirected()`, and the thing a tool
checks to skip work. Two rules taken straight from Hilbish's mistakes: the sinks are bound to
the *real* fds this stage got, never to buffers (their buffered sinks silently ate output,
issues #344/#352), and nothing in the read-eval loop or the alias/word resolution moves into Lua
(their `MustDoString` per command, per pipeline stage, is why Hilbish is slow).

CONSUMING. Two faces, same data. Direct call, unchanged: `local rows = sh.df()`. Inside a tool:
`for row in io.input:rows() do`. And in shell mode the consumers take a **Lua expression**,
because oslo already has a Lua evaluator in-process and inventing a filter language would be the
exact thing the owner said not to do:

```sh
df | where 'free < 1gb' | cols mount free | sort-by free
ps | where 'cpu > 10' | each 'print(row.name)'
```
`where` evaluates its argument with the record's columns bound as locals, plus `row` as the
whole record. That is nushell's bare-field shorthand (`where type == dir` meaning `$it.type`)
achieved with a Lua scope rather than new syntax — and it is strictly better than theirs,
because the escape hatch (`each`) is the same language as the shorthand, so there is no cliff.
If a user has to write `where {|r| r.type == "dir"}` they will type `grep dir` instead and the
structured channel dies unused.

`each` is the pressure valve and must ship on day one: it is what stops users demanding operator
number forty, and its marginal cost is an adapter, not an interpreter, because the evaluator is
already there.

ONE PREREQUISITE, worth paying now. `Table`'s hash part is a `HashMap`
(src/lua/eval/value.rs:150), so a record converted to a Lua table loses column order and
`pairs()` iterates differently on every run. Swap it for an insertion-ordered map. Lua leaves
`pairs` order unspecified, so this is legal and strictly better; it fixes a nondeterminism oslo
has today; and it is a contained change in one 394-line file now versus a cross-cutting one
after twenty tools exist. The renderer should still take column order from `Val::Record` and
never ask Lua for it, but the Lua face should not be lying either.

## Transport

**In-process only for v1. Cross-process structure is deliberately out of scope, with one door specified so it is not a break when it opens.**

Why in-process is not a cop-out: between two structured stages oslo does not fork at all, so
there is no serialisation, no descriptor, no format, no framing, no version skew, and no
compatibility promise. It is simultaneously the fastest design and the smallest one. The
question "how does structure cross a process boundary" is answered by not crossing one.

Why not now: the moment structure crosses a process you own a wire format, a version
negotiation, framing, backpressure and a permanent promise to every tool in the distro. That is
nushell's plugin protocol — a `Hello` handshake, a `features` array, msgpack framing, stream
ids, `Data`/`Ack`/`End`/`Drop` — and it only ever works for programs written for nushell. It is
a large permanent surface bought to solve a problem oslo does not have on day one, because day-
one structured tools are all in the binary. `docs/built-in-tools.md` already left "one binary or
a separate crate" open; this is the same question and the same answer: decide it when a tool's
weight is measurable.

THE DOOR, specified now:
- Reserve `OSLO_DATA_FD` and `OSLO_DATA_FMT`. When an external participates, the shell opens an extra pipe, dup2s the write end to a free descriptor n, exports `OSLO_DATA_FD=n` and `OSLO_DATA_FMT=json-seq`, and runs the program normally. The program writes records there and its human text to stdout as always. This is the `tree`/`STDDATA_FD` convention (rgbcu.be/blog/3-json/), and it is the only shape that survives contact with POSIX: no handshake, no linkage, degrades to silence, stdout byte-identical.
- The variables are exported **only** when `plan()` chose `Rows` for that edge, which for an external requires an explicit declaration — `oslo.tool.declare{name="mytool", produces="rows"}` in config, or a file at `/usr/share/oslo/tools/<name>.lua` that the distro's package ships. Never for an unknown external. That is what keeps the "no observable environment change" argument true.
- Format: **JSON text sequences, RFC 7464** — `\x1e` + one JSON value + `\n` — not msgpack, and not a bespoke format. serde_json is already a dependency (src/lua/api/json.rs, with a documented reason: lua-cjson cannot be dlopened on static musl). One framing byte. A human can debug it with `cat <&3`. A tool author in any language emits it in three lines. msgpack buys throughput a shell does not need and costs every tool author a library dependency — which for a distro means costing every *package* a dependency.
- Backpressure is the pipe. The kernel already provides it. No `Ack` protocol: nushell needs one because their transport is bidirectional RPC over a single duplex channel; a one-way record stream on a pipe needs nothing. Early consumer exit is SIGPIPE, which already terminates the producer correctly — that is nushell's `Drop` message, for free.
- Negotiation is one rule: if `OSLO_DATA_FMT` names something the shell does not know, the shell does not open the descriptor. No `Hello`, no version field, no feature array in v1. Version by adding a new value for `OSLO_DATA_FMT`.

What this means concretely: v1 ships a dual-channel pipe where the second channel is an iterator
in one process. The second channel between processes is a documented, reserved, unimplemented
extension. That is the honest answer, and it is better than shipping a half-designed protocol
that the distro then has to live with.

## What ships first

- df — already 80% written (src/lua/api/tools.rs parse_df), bounded, its text form is genuinely
  hard to parse (mount points with spaces), and it exercises every part of the shape: two faces,
  named fields, a tagged scalar, and arguments that change the text face without changing the
  rows
- where — the operator that justifies the whole pipe; Lua expression with columns bound as
  locals
- from json — turns every tool with a --json flag into a structured source, with no cooperation
  from anyone
- to json — the exit door; without it the structured world is a dead end the moment someone
  wants jq or curl
- each — the pressure valve; stops the demand for operator number forty, and costs an adapter
  rather than an interpreter because the Lua evaluator is already in-process
- lines and parse — the rest of the text-to-structure bridge, and the implementation strategy
  for every later tool given oslo's stated decision to parse external output rather than
  reimplement it
- cols, get, sort-by, first — trivial once Val and Record exist, and `ps | where 'cpu > 10' |
  sort-by cpu | first 5` is the sentence that sells the feature

## Staging

Each stage is independently useful, so the work can stop between any two of them.

1. Stage 0 — prerequisite, one file. Make `Table`'s hash part insertion-ordered
   (src/lua/eval/value.rs:150). Independently useful: it makes `pairs()` deterministic, which is
   a bug oslo has today. Hard prerequisite for records having stable column order.
2. Stage 1 — `src/data/`: `Val`, `Record` (ordered), `Data`, `RowStream` with the interrupt
   flag, `DataError`, and the TWO renderers (`render_transport`, `render_display`) as separate
   functions from the start. No pipeline changes at all. Independently useful: rewrite
   `sh.df()`/`sh.ps()`/`sh.ls()` (src/lua/api/tools.rs) to build `Val` and convert at the Lua
   boundary — this proves the model against tools that already exist and already have tests, and
   gets `Val::Size` replacing the hand-rolled `size`/`size_human` pairs.
3. Stage 2 — the `Tool` registry with `accepts`/`produces`/`collects`, and `plan()`.
   `run_stages` gets its one new branch, which is a no-op until a structured tool exists.
   Independently useful as pure risk reduction: land it with the 375-script differential corpus
   green AND the instrumented assertion that the corpus never enters the structured path. This
   is the commit that makes the POSIX claim mechanical.
4. Stage 3 — the first real pipeline: `df` as a structured tool plus `where` as a structured
   consumer, plus the no-fork collapse in `run_stages` and the synthesised PIPESTATUS vector.
   `df | where 'free < 1gb'` works. This is the smallest thing that is genuinely useful and it
   is the demo.
5. Stage 4 — the bridge INTO structure, which is what makes the feature useful on today's
   machine rather than after the distro's tools exist: `from json`, `lines`, `parse '{a} {b}'`.
   `kubectl get pods -o json | from json | where 'phase == "Running"'` with zero oslo-aware
   programs involved. Arguably this outranks Stage 3 in value; it cannot precede it because it
   needs a consumer to feed.
6. Stage 5 — the exit door and the rest of the day-one verb set: `to json`, `to text`, `cols`
   (NOT `select`), `get`, `sort-by`, `first`/`last`, `length`, `each`. Each is under a day once
   Stage 1 exists. `to json` must be in this batch or the feature is a walled garden the moment
   someone wants jq.
7. Stage 6 — `oslo.register_tool` with the `io`/sinks object, and the widened `register_builtin`
   signature. Deliberately last: the Lua API should be a face on a proven Rust shape, not a
   guess that then constrains the shape. The one thing that must NOT wait is the decision to
   widen the signature — announce it in Stage 0 so nobody builds on `function(argv)`.
8. Deferred on purpose, and named so they are not rediscovered as gaps: `OSLO_DATA_FD` cross-
   process transport, an out-of-process plugin protocol, an `explore`-style TUI pager, `par-
   each`, duration/date arithmetic beyond comparison, and anything columnar. Nushell spends 193
   of 564 commands on dataframes; a shell that replaces /bin/sh does not need Arrow.

## Still open

- `select` is unavailable as a tool name. src/parser/brush_adapter/mod.rs:19 refuses `select` by
  name because it is a bash keyword, so `df | select mount` will not parse. Use `cols` or
  `pick`. Worth deciding before any docs are written, because nushell users will reach for
  `select` by reflex.
- How does `where 'free < 1gb'` get a `1gb` literal? Lua has no size suffixes. Options: extend
  oslo's own Lua lexer with `1gb`/`2mb`/`30s` suffixes (oslo owns the lexer, and this is the
  same decision nushell made); require `oslo.size'1G'` (ugly enough that people will not use the
  feature); or a tiny non-Lua expression dialect (violates the no-new-language rule). I lean to
  extending the lexer, but it is a real language change and should be argued explicitly.
- PIPESTATUS across a collapsed run. `set_pipeline_status` (src/env/scope.rs:224) must receive
  the same-length vector it would have for a forked pipeline, with a per-stage status
  synthesised inside one process. Where exactly a collapsed stage's status is decided — and what
  a mid-stream `Val::Error` does to it — needs settling before Stage 3.
- What decides `Sink::Print` versus `Sink::Text` for the last stage: 'fd 1 is a tty' or 'no
  redirection applied'? They differ for `df > f` and for `oslo -c 'df' > f`. The tty test is
  what users expect for colour; the redirection test is what is predictable in scripts. Probably
  both, with colour keyed to the tty and layout keyed to redirection.
- Does `sh.df()` and the `df` tool share one implementation, or do they drift? `docs/built-in-
  tools.md` says `run` is defined in terms of `rows`; the registry has to make that structurally
  true rather than conventionally true, or there will be two `df` parsers within a year.
- Should the collapse (structured stages sharing one process) be observable to the user? A
  pipeline where a variable assignment survives across a `|` is a genuine divergence from every
  other shell, even if no existing script can reach it. Either document it loudly or forbid
  assignment-visible effects in structured tools.
- Where does an accumulated `Val::Error` surface? Nushell decides at drain time. oslo has `$?`,
  PIPESTATUS and stderr — the mapping from 'three of forty rows failed' onto those three
  channels is not obvious and should be decided once, in Stage 1, not per tool.
- Whether `Val` should carry a span for diagnostics. Nushell is actively retreating from spans-
  on-Value because a span per scalar is too expensive. Put spans on the pipeline STAGE, not on
  cells — but the two-labelled-span error format (point at the operator AND at the offending
  operand) is worth copying, and oslo already emits OSC 8 hyperlinks in diagnostics so half the
  rendering exists.
