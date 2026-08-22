# A calculator that knows units

`math` works out an expression, and the units come with it:

```sh
$ math '3 km in miles'
1.86411357671 miles
$ math '5 GB / (40 MB/s) in minutes'
2.08333333333 minutes
$ math '10 kg * 9.81 m/s^2'
98.1 kg·m·s⁻²
```

> ## This is in `oslo`, not in `oslo-minimal`
>
> Everything on this page is behind the **`math`** cargo feature, which is off by default.
>
> | | |
> |---|---|
> | `oslo` | the `math` builtin and `oslo.math` |
> | `oslo-minimal` | neither, and the word `math` falls through to `$PATH` |
>
> ```sh
> scripts/build.sh              # the full binary, every feature
> scripts/build.sh --minimal    # no calculator
> ```
>
> It costs **96 KB** — 6,222,624 bytes without it against 6,320,928 with. It is off because a
> `/bin/sh` on a distribution has no business converting furlongs, not because of the size.

<!-- demo:begin -->
[![math demo](https://asciinema.org/a/1263448.svg)](https://asciinema.org/a/1263448)
<!-- demo:end -->

## How it works

An expression is parsed into a number and a **dimension** — not a unit name, a dimension: length,
time, mass, or a product of them. That is what makes the arithmetic mean anything. `10 kg * 9.81
m/s²` is not string concatenation; the answer carries `kg·m·s⁻²` because mass times acceleration is
what that is, and nothing declared a "newton" for the occasion.

```
  '5 GB / (40 MB/s) in minutes'
        │
        ├─ parse ──► 5 × 10⁹ [mass⁰ length⁰ time⁰ …]  ÷  4 × 10⁷ [time⁻¹]
        │
        ├─ arithmetic on the dimensions as well as the numbers ──► 125 [time]
        │
        └─ `in minutes` ──► is `minutes` a time?  yes ──► 2.0833… minutes
                                                 no  ──► refused, naming both kinds
```

The conversion at the end is checked against the dimension rather than a table of pairs, so
`3 km in miles` and `3 km in seconds` fail differently: the first converts, the second is refused
because length is not time. A calculator that let it through would give a number nobody could use.

### Left to right, like every other calculator

`5 GB / 40 MB/s` is **125 s⁻¹**, not 125 s. Division associates left to right, so it reads as
`(5 GB / 40 MB) / s` — which is what `bc`, `python` and a pocket calculator all do with `a/b/c`.
Write the parenthesis when you mean the rate: `5 GB / (40 MB/s)`.

This is worth knowing precisely because the wrong reading still produces an answer. The dimension is
the tell — `s⁻¹` where you expected `s` means the `/s` bound somewhere else.

### The three ways to ask for less

The full answer is a number and a unit. A script usually wants one or the other:

```sh
$ math -v '3 km in miles'     # 1.86411357671    the number alone
$ math -u '3 km in miles'     # miles            the unit alone
$ math -k '5 GB / (40 MB/s)'  # time             what kind of thing it is
```

`-k` is the one to reach for when an expression surprises you: it answers with the dimension —
`length`, `time`, `length·time⁻¹` — rather than the unit, so a wrong reading shows up as a wrong
*kind* before you have to think about the number.

### Nothing remembers anything

```sh
$ math 'x = 5'
oslo: math: nothing here remembers x — a session does: oslo.math.session()
```

Each `math` is its own process with its own empty scope, and an assignment that vanished silently
would be worse than one refused. A scope that persists is a Lua object, asked for by name:

```lua
local s = oslo.math.session()
s:eval("rate = 40 MB/s")
s:eval("5 GB / rate").text     --> "125 s"
s:names()                      --> { "rate" }
```

## What makes it different

`bc` has arbitrary precision and no units; `units(1)` has units and no arithmetic worth the name;
`qalc` has both and is a 3 MB package with a library behind it. This is 96 KB inside a shell that is
already running, so `math '3 km in miles'` costs a process spawn and nothing else — 1.87 ms against
1.84 ms for `oslo -c true`, which is to say the calculator itself is lost in the noise of starting a
process at all.

The dimension check is the part `bc` cannot have. `3 km + 2 s` is not a number in this calculator;
it is a question with no answer, and it says so.

## Configuration

None. It is a builtin and a Lua table; there is nothing to set.

## Measurements

| | |
|---|---:|
| units known | 159 |
| functions known | 30 |
| `math '3 km in miles'`, whole process | 1.87 ms |
| `oslo -c true`, whole process | 1.84 ms |
| `echo '3*1.609' \| bc`, whole process | 2.15 ms |
| the feature's cost in the binary | 96 KB |

`math --units` and `math --functions` print both lists.

## What it cannot do

- **Remember anything between invocations.** Each `math` is a process. `oslo.math.session()` is the
  scope that persists, and it lives in Lua rather than in the builtin.
- **Guess what you meant by `a/b/c`.** Division is left-associative; the dimension in the answer is
  how you notice.
- **Convert between kinds.** `3 km in seconds` is refused rather than answered, which is the whole
  point of carrying dimensions.
- **Define a unit.** The 159 it knows are the 159 it knows; there is no way to add one from a
  config today.
- **Do arbitrary precision.** These are floats. `bc` is the tool when the last digit matters.

## Where it lives

| path | what is in it |
| --- | --- |
| `crates/oslo-math/src/lib.rs` | `calculate`, `calculate_in`, `Scope` — the entry points |
| `crates/oslo-math/src/parse.rs`, `eval.rs` | the grammar and the evaluation |
| `crates/oslo-math/src/dimension.rs`, `units.rs` | what makes `3 km + 2 s` a refusal |
| `crates/oslo-runtime/src/lua/api/math.rs` | `oslo.math` — `eval`, `value`, `convert`, `units`, `functions`, `session` |
| `crates/oslo-shell/src/env/builtins/math.rs` | the `math` builtin and its flags |
