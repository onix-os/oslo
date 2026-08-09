# Splitting oslo into six crates

Written outside the repository on purpose — the working tree is about to be disturbed.

Target: `src/` becomes a binary of about 900 lines that parses argv and calls a library. Everything
else lives in six crates. Not fifteen; the earlier fifteen-crate sketch was wrong and is dropped.

---

## What the code actually says

Every number below was measured on the tree at `slim/footprint`, counting **code only** — an
earlier pass counted doc comments as dependencies and produced a graph twice as tangled as the
real one. Three measurements decide the whole design.

**1. `lua/eval` is already a standalone interpreter.**

```
lua/eval → ui:  0        lua/eval → env: 0
```

Not "nearly zero". Zero. It is packaged as part of the shell but does not depend on it.

**2. `env/scope` — the `Environment` — depends on nothing.**

```
env/scope → exec: 0   → expand: 0   → parser: 0   → lua: 0   → ui: 0
```

Every outward edge attributed to `env` in a naive count comes from `env/builtins/`, not from the
variable store. The store drops to the bottom of the graph untouched.

**3. `exec` and `builtins` cannot be separated.**

```
exec → env::builtins:  13
```

Running a simple command dispatches to a builtin; builtins call back into `exec` for subshells and
`eval`. This is mutual recursion in the problem, not in the code. They are one crate. That is why
`oslo-shell` is large and no amount of rearranging makes it smaller.

**4. Everything below the Lua layer touches it only for hooks — 23 references.**

```
crate::lua::api::hooks::at::*         10     the hook-name constants
crate::lua::engine::fire_at_here       7
crate::lua::engine::answer_hook_with   2
crate::lua::engine::ask_hook_here      1
crate::lua::api::hooks::watched        1
crate::lua::api::prompt                1     the one outlier
```

That is the entire reason `ui`, `exec` and `builtins` appear to depend on Lua. It is a façade, and
moving it is the only architectural change this plan requires.

---

## The six

Each depends only on those below it. There are no cycles.

| # | crate | lines | contents | depends on |
|---|---|---|---|---|
| 6 | `oslo` (bin) | ~875 | `main.rs`, `cli.rs` | 5 |
| 5 | `oslo-runtime` | ~11,400 | `lua/api`, `lua/engine`, `startup` | 1–4 |
| 4 | `oslo-shell` | ~36,700 | syntax adapter, `expand`, `exec`, `builtins`, `data`, `direnv` | 1–3 |
| 3 | `oslo-ui` | ~30,100 | line editor, completion, dropdown, widgets, theme, finder | 1, 2 |
| 2 | `oslo-base` | ~10,900 | `ast`, `error`, `feature`, `Environment`, `track`, `ssh`, hook registry | 1 |
| 1 | `oslo-lua` | ~4,826 | `lua/eval` — the interpreter | — |

`oslo-lua` is worth having as a crate for its own sake. It is a Lua evaluator in pure Rust with no
dependencies at all; `full_moon` parses, this runs. That combination is why oslo can speak Lua in a
static musl binary with no C toolchain, and it is publishable on its own.

---

## The one change: a hook registry in `oslo-base`

Today `ui`, `exec` and `builtins` call `lua::engine::fire_at_here` directly. That is what pins the
Lua layer underneath them while `lua/api` needs to sit above them.

Invert it. `oslo-base` gains a registry with no dependencies:

```rust
pub mod hooks {
    pub mod at { pub const PRE_PROMPT: usize = 0; /* … */ }

    /// Set once, by oslo-runtime, at startup.
    pub fn install(fire: fn(usize, Vec<Value>), ask: fn(usize, Vec<Value>) -> Option<Value>);

    pub fn fire_at_here(index: usize, args: Vec<Value>);
    pub fn answer_hook_with(index: usize, args: Vec<Value>) -> Option<Value>;
    pub fn watched(index: usize) -> bool;
}
```

Roughly 150 lines. `oslo-runtime` calls `install` during startup; before that, firing a hook is a
no-op, which is already true for `sh -c` and for scripts.

Two consequences worth stating:

* `Value` is `oslo_lua::Value`, so `oslo-base` depends on `oslo-lua`. That edge already exists —
  `ShellError::Lua(LuaError)` — and it points downward, so it costs nothing.
* `crate::lua::api::prompt` (one reference, from `ui`) is not a hook. Handle it separately: either
  move that caller up or add a second registry slot. Do not let it force the whole prompt module
  into `oslo-base`.

**`expand → exec` needs no trait.** The earlier plan proposed inverting command substitution behind
a `Substitute` trait. With `expand` and `exec` in the same crate, those two call sites are ordinary
function calls and the trait is unnecessary. Do not build it.

---

## Do these first

Neither is part of the split, and both make it smaller.

**Delete `src/lexer/` (2,565 lines).** It exists because brush's tokenizer returns
`Token::Word(String, span)` — the raw source text — and discards the word's structure, which the
expander needs. `parser/brush_adapter/words.rs` re-lexes that text with oslo's own lexer; the
function's own doc says "re-lexing is the bridge". It is also where the parse-time OOM hang lived.

brush-parser is vendored now, and it already has `word::parse() -> Vec<WordPieceWithSource>` with
`Text`, `SingleQuotedText`, `DoubleQuotedSequence`, `ParameterExpansion`, `CommandSubstitution`,
`EscapeSequence` — the same concepts as oslo's `WordPart`. The information is not missing; it is
thrown away at the tokenizer boundary and rebuilt by a second lexer.

Replace the bridge with a `WordPiece → WordPart` conversion. Estimated 400–700 lines replacing
2,565. The hard part is oslo's `ParamExpansion` against brush's richer `ParameterExpr`; the rest is
mechanical.

**Move alias expansion out of `src/parser/`.** `alias.rs` + `alias/` is 1,146 lines of shell
feature sitting in what is otherwise an AST adapter. It belongs with the builtins.

After both, `src/parser/` is the brush→oslo adapter plus the nesting guard, and deserves the name
`syntax` rather than `parser`. There is one shell parser and it is brush's.

---

## Order

Each step ends with `make verify` green and is its own commit.

1. **Hook registry into `oslo-base`'s future home** (still inside `src/` — no crate yet). The only
   risky change; do it alone, while everything is still one crate and the compiler can see all of it.
2. **`oslo-lua`** — move `lua/eval`. Smallest, cleanest, zero dependencies. Proves the workspace
   plumbing before anything large moves.
3. **`oslo-base`** — `ast`, `error`, `feature`, `env/scope` + `env/options`, `track`, `ssh`, hooks.
4. **`oslo-ui`** — the largest single move, but by now everything it needs exists below it.
5. **`oslo-shell`** — syntax, expand, exec, builtins, data, direnv, in one move. Splitting this
   step is what the `exec ↔ builtins` measurement says you cannot do.
6. **`oslo-runtime`**, then strip `src/` to `main.rs` and `cli.rs`.

Steps 2–6 are import churn: `crate::ui::` becomes `oslo_ui::` across nearly every file. Mechanical,
large, and reviewable only in the sense that the compiler checks it.

---

## Things that will bite

**`check-loc` scans `src tests examples`.** After step 2 most of the code is not in `src/`, and the
600-line rule silently stops applying to it. Fix `scripts/check-loc.sh` in the same commit as the
first extraction, or the rule quietly dies.

**`make verify` becomes workspace-wide.** `clippy --all-targets --all-features` already is. The
differential corpus still needs the built binary, so that target stays pointed at the bin.

**Vendored crates and yours share `crates/`.** `crates/` currently means "somebody else's code, kept
close to upstream, not restyled" — that is what `crates/README.md` says and what the lint allows at
the top of `full_moon/src/lib.rs` mean. Putting oslo's own crates beside them destroys that
distinction. Move the two parsers to `vendor/` first, or put oslo's crates somewhere else.

**In-tree tests move with their modules.** About 11,500 lines of `#[cfg(test)]` travel with the code
and need no work. The 16,300 lines in `tests/` stay with the binary and switch to `oslo_ui::`-style
paths.

**Compile time may get worse before it gets better.** More crate boundaries mean less cross-crate
inlining; full rebuilds can slow while incremental rebuilds improve a lot. Related: this tree is
measurably sensitive to LTO layout — removing an unrelated dependency moved a hot loop 4–6% in both
directions during the dependency work. Measure with min-of-N on a quiet machine, interleaved, or
the numbers are noise.

**Naming.** `oslo-lua` is plausibly publishable; the other four are not. Either name them all
`oslo-*` for consistency, or name the publishable one `oslo-lua` and the rest plainly. Decide before
step 2, because renaming later is another sweep through every import.

---

## What this does not do

It does not make the code smaller. The six crates hold the same ~95,000 lines, minus the ~2,600 of
`src/lexer/` and whatever the comment budget gives back. Halving the codebase was examined
separately and is not reachable without dropping a feature — the earlier finding stands: this is a
POSIX shell, a Lua interpreter, a TUI toolkit, a history database and a structured pipeline in one
repository, and the arithmetic does not work out any other way.

What it does buy: a binary that is 900 lines instead of 95,000, a Lua interpreter that can be used
on its own, and boundaries the compiler enforces instead of ones that live in a style guide.
