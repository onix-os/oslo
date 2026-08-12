# Suggestions you can plug into, and tune

Two things the shell offers as you type — the **ghost** past the cursor and the **dropdown** on Tab —
are closed. A config can reorder the ghost's four built-in sources and replace the dropdown for one
named command; it cannot add to either, and it cannot say *how* two answers should compete.

This opens both, and the second half is the point: **who wins is the user's decision, not the
plugin's.** If you want your LLM to beat the model, say so and it does.

**Work on a new branch off `develop`.**

## Why now

Two plugins nobody can write today:

- **tldr** — 3,000 pages of what people actually do with a command. `git <Tab>` should offer them.
  Half is already possible (`oslo.completion.spec` takes tldr's shape); what is missing is *adding*
  to a command oslo already knows, and answering generically rather than one name at a time.
- **an LLM ghost** — regenerated as you type. Nothing about this fits: the sources are a Rust enum
  and the call is synchronous on the keystroke path, where a 300 ms answer is not an answer.

The slow one decides the design. **A provider that can be slow is the general case.**

## What other systems do

Six of them, and each is right about something different.

| | inline / ghost | menu / dropdown | how two answers compete |
|---|---|---|---|
| **zsh-autosuggestions** | `ZSH_AUTOSUGGEST_STRATEGY` array — *"tried successively until a suggestion is found"* | — | first to answer wins |
| **zsh compsys** | — | `completer` style: an ordered list, the next tried when the previous found nothing | ordered fallback, plus `tag-order` / `group-order` for presentation |
| **VS Code / Monaco** | `InlineCompletionsProvider.groupId` + `yieldsToGroupIds` | `CompletionItemProvider` | a provider **declares** it yields to another group |
| **nvim-cmp** | — | `group_index` — a higher group is ignored entirely if a lower one produced anything; `priority` within a group | fallback tiers |
| **blink.cmp** | — | per provider: `enabled`, `async`, `timeout_ms`, `min_keyword_length`, `max_items`, `score_offset`, `fallbacks`, `transform_items` | a numeric **nudge**, plus explicit fallbacks |
| **Emacs** | — | `completion-at-point-functions` — sequential, first that answers wins; `cape-capf-super` **merges** several so they appear together | two named modes: sequential *or* merged |

Five lessons worth taking:

1. **The two surfaces need different composition.** Emacs makes it explicit: a list is sequential by
   default, and merging is a thing you ask for by name. The ghost can only draw one string, so it is
   first-wins. A menu shows everything, so it is merged. oslo already behaves this way by accident;
   this makes it deliberate.
2. **Tiers beat a flat list.** nvim-cmp's `group_index`, zsh's `completer`, blink's `fallbacks` are
   the same idea: *only ask the next tier if nothing better answered*. This is the shape of the LLM
   question — put it in a tier and let the user choose the tier.
3. **VS Code has the right control in the wrong hands.** `yieldsToGroupIds` is declared by the
   *provider*. That is exactly the complaint: the plugin decides, you do not. Here a provider
   declares a **default** and the config overrides it.
4. **A nudge is finer than an order.** blink's `score_offset` composes with ranking instead of
   replacing it — and oslo already sorts by frecency (`completion.rs:214`), so an offset drops
   straight in.
5. **zstyle is the deepest fine control anyone has shipped.** Its context is
   `:completion:function:completer:command:argument:tag`, and any style can be set for any pattern —
   so "for `git`, at argument 1, prefer these" is one line. That axis-per-colon idea is what
   "more fine control" actually means; the syntax is not.

## What oslo has

**The ghost.** `OsloHelper::suggest` (`crates/oslo-ui/src/lib.rs:177`) walks `oslo.suggest.sources`
and takes the first that answers, over a closed enum: `History | Completion | Path | Prediction`.
Called from `Assist::hint_text` → `frame::draw`, synchronously, once per keystroke. Guarded by three
rules worth keeping: end of line only, `sh` only, `feature::on(SUGGEST)`.

**The dropdown.** `OsloHelper::candidates` merges the built-in builders, then
`oslo.completion.sources` filters by **kind** (`completion.rs:203`), then it sorts by frecency and
name (`completion.rs:214`). `for_command` (`completion.rs:116`) **replaces** everything for one named
command and supplies **no kind**, so `sources` filters its candidates out entirely — a known hole.

**The discovery that shapes the plan.** The editor already knows how to wait for an answer that has
not arrived — built for asynchronous *prompts*. `frame::next_input` (`edit/session/frame.rs:22`):

```rust
if !idle_hook && !prompt::refreshing() { return keys.read_event(); }   // block: the default
const REFRESH_SLICE_MS: i32 = 15;
Timeout => if prompt::generation() != seen { return Some(InputEvent::PromptRefreshed); }
```

An outstanding count, a generation counter, a 15 ms slice used *only* while something is outstanding,
and a synthetic event that makes the loop redraw. And `lua/api/external.rs` states the constraint in
its own words — *"a prompt is on the critical path of every keystroke"* — answered with `timeout_ms`
plus an `async` mode that uses the previous answer immediately.

**So this is mostly generalisation.**

## The design

### 1. One channel for a late answer

Lift `prompt::refreshing`/`generation` into `crates/oslo-ui/src/pending.rs`: an outstanding count and
a generation, with the prompt as its first caller and the ghost and dropdown as the next two.
`next_input` waits in slices while *anything* is outstanding. ~60 lines, and nothing below works
without it.

### 2. Tiers, not a flat list

```lua
oslo.suggest.sources = {
  { "history", "predict" },   -- tier 1: instant, local, and only ever something you really ran
  { "llm" },                  -- tier 2: asked only when tier 1 said nothing
  { "path" },                 -- tier 3
}
```

A tier is consulted only if every earlier tier answered nothing — nvim-cmp's `group_index`, zsh's
`completer`. **The flat list stays valid** and means one tier per entry, so every config written
today keeps working.

Want the LLM to beat history? Put it in tier 1 and give it the offset, or put history in tier 2. It
is one line either way, and it is *your* line — the plugin does not get a vote.

### 3. What happens when the slow one is late — the user decides

This is the real question. `predict` answers in ~4 µs, an LLM in ~300 ms. Something is already drawn
when the slow answer lands.

```lua
oslo.suggest.provider {
  name = "llm",
  on_late = "replace",     -- "fill" (default) | "replace" | "drop"
  settle_ms = 400,         -- after this, too late to swap under your eyes: treated as "drop"
}
```

- **`fill`** — draw only if nothing is drawn. Nothing ever changes under the cursor. The safe default.
- **`replace`** — swap it in. Text you are looking at changes half a second after you stopped typing.
  It is a real thing to want, and if you say so, that is what happens.
- **`drop`** — never draw after the fact; the answer is only used if it arrives before the frame.

`settle_ms` is the guard that makes `replace` liveable: an answer that took four seconds is not
allowed to rewrite a line you have moved on from.

### 4. A nudge for the dropdown

The menu merges rather than choosing, so tiers are the wrong tool. `score_offset` is the right one —
added to the frecency score in the existing sort:

```lua
oslo.completion.provider { name = "tldr", kind = "example", score_offset = 20 }
```

`fallbacks = { "…" }` covers the other case blink identified: *if I return nothing, ask these*.

### 5. Context rules — zstyle's lesson, in Lua

One order for every situation is not fine control. The axes oslo actually has are the command, the
argument position, the language, and where you are standing:

```lua
oslo.suggest.rules = {
  { when = { command = "git" },        use = { { "llm" }, { "history" } } },
  { when = { language = "lua" },       use = { { "history" } } },   -- no model, no LLM, in Lua
  { when = { cwd = "~/work/*" },       use = { { "history" } } },   -- nothing leaves this tree
}

oslo.completion.rules = {
  { when = { command = "git", arg = 1 }, offset = { tldr = 50 } },
  { when = { kind = "file" },            offset = { tldr = -100 } },
}
```

First matching rule wins, falling back to the global setting. `when` fields are all optional and
ANDed; `command` and `cwd` take the shell's own glob. This is `zstyle ':completion:*:*:git:*' …`
without the colons — the same power, in the language the rest of the config is written in.

### 6. Guards, all borrowed

Per provider, and every one of them earns its place from somebody else's experience:

| | from | why |
|---|---|---|
| `min_chars` | blink `min_keyword_length` | do not ask a model about `g` |
| `max_line` | `ZSH_AUTOSUGGEST_BUFFER_MAX_SIZE` | a pasted 4 KB line is not a prompt for suggestions |
| `ignore = { "…" }` | `ZSH_AUTOSUGGEST_HISTORY_IGNORE` | glob patterns that silence the provider — and the privacy control |
| `enabled = function(ctx)` | blink, nvim-cmp | anything the other fields cannot express |
| `debounce_ms` | every LLM plugin | ten keystrokes in one word is one request, not ten |
| `timeout_ms` | blink, `external.rs` | and a **sync budget**: a provider that overruns repeatedly is disabled for the session and says which it was |
| `max_items` | blink | one provider cannot flood the menu |

### 7. The continuation invariant, and trust

**The ghost is drawn as trailing text and accepted with Right**, so an answer that is not a
continuation of the line would make that key lie. Such an answer is **refused and reported once**
through `messages`, never trimmed into something the plugin did not write. A provider that wants to
*replace* the line is asking for the repair slot, which this plan leaves alone.

**A ghost provider sees every keystroke** — including the ones you retyped because a password went
into the wrong field. An LLM provider sends them off the machine. So:

- `oslo plugin doctor` **names every provider that can see typing**, because "what reads my
  keystrokes" must have an answer that is not "read the source";
- `oslo.feature` can kill them mid-session (`SUGGEST` already exists);
- they are not asked at all when the line is secret, under the leading-space convention, or when
  `no_trace` is set. The veto work established that every sink asks one flag; this is one more sink.

## The API, whole

```lua
-- ghost: fast
oslo.suggest.provider { name = "tldr", answer = function(ctx) return "…" end }

-- ghost: slow
oslo.suggest.provider {
  name = "llm", tier = 2, on_late = "fill", settle_ms = 400,
  debounce_ms = 120, timeout_ms = 2000, min_chars = 3, max_line = 512,
  ignore = { "* --password *" },
  request = function(ctx, reply)
    oslo.spawn { "llm", ctx.line, on_exit = function(out) reply(out) end }
  end,
}

-- dropdown
oslo.completion.provider {
  name = "tldr", kind = "example", score_offset = 20, max_items = 10,
  answer = function(ctx)          -- ctx = { command, words, current, arg, cwd, language }
    return { { display = "git commit --amend", description = "change the last commit" } }
  end,
}
```

`ctx` for the ghost is `{ line, cursor, cwd, language, last_status }`. Returning `nil` declines and
the next source is asked — exactly what the built-in sources do.

## Order

Each step ends with `make verify` green and is its own commit.

1. **`pending`** — lift the outstanding/generation pair out of `prompt.rs`; `next_input` waits on any
   of them. No behaviour change; the prompt tests are the proof.
2. **Ghost providers, sync.** Registry, the fifth source, the continuation invariant, the sync
   budget. Measured against `bench/keystroke.rs` with none installed.
3. **Ghost providers, async.** `request`/`reply`, debounce, staleness by generation, `timeout_ms`.
4. **`on_late` and `settle_ms`.** The three policies, and the test that `fill` never changes drawn
   text.
5. ~~**Tiers** for `oslo.suggest.sources`.~~ **Dropped, on inspection.** The ghost takes the first
   source that answers, so `{{a, b}, {c}}` — try `a` and `b`, and only if both declined try `c` — is
   exactly what the flat `{a, b, c}` already means. Grouping is worth having where results *merge*,
   which is why nvim-cmp has `group_index` for its menu; a surface that can draw only one string has
   nothing to group. It belongs to the dropdown, and it is in step 6.
6. **Completion providers**, sync then async: additive, kinds, `score_offset`, `fallbacks`,
   `max_items`. Fixes the `for_command` no-kind hole on the way past.
7. **Context rules** for both, and the guards table.
8. **Two worked examples** in `examples/plugins/`: `tldr` (sync, `oslo.db`-backed) and `slowpoke`
   (async, `oslo.spawn`-backed — an LLM stand-in that is slow and *deterministic*, so it can be a
   test).

Steps 1–6 are core and work in `oslo-minimal`; nothing here is behind a cargo feature, because the
ghost and the dropdown are not.

## Verification

- `bench/keystroke.rs` **before and after step 2**, min-of-N: a shell with no provider installed must
  not pay for the mechanism. If it does, the lookup goes behind an "any providers at all" atomic.
- **A stale answer is never drawn.** Type, request, type again, answer the first — the frame shows
  the second line's suggestion or none, never the first's.
- **`on_late = "fill"` never changes drawn text**, and `"replace"` does, and `settle_ms` stops it.
- **A non-continuation is refused**, and reported once rather than per keystroke.
- **A slow sync provider is disabled** rather than paid for on every key.
- The async paths belong in the pty tests (`tests/terminal_semantics/`) — a ghost that repaints is
  only observable in a real editor.
- `oslo-minimal` builds, and its ghost still works with no providers registered.

## Things that will bite

- **`edit/session.rs` is 595 lines, `completion.rs` 588, `settings/from_lua.rs` 544.** All three
  cross 600 on the first step that touches them. Split by subject before adding.
- **Reentrancy.** `config_candidates` documents it (`completion.rs:121`): the hook runs Lua, Lua can
  complete another word, and the outstanding `RefCell` borrow panics. Every new registry needs the
  same clone-before-call.
- **Lua is not `Send`.** `answer` runs on the shell's thread; only the *work* an async provider
  starts leaves it. That is why `request`/`reply` is shaped like `oslo.spawn` and `reply` is
  delivered at a safe point, never from the worker.
- **`InputEvent::PromptRefreshed` is named for the prompt** — renaming touches the editor tests.
- **The dropdown owns the terminal.** Rows arriving while it is open must not move the selection: a
  row inserted above the cursor means Enter runs something you did not choose.
- **Settings are read two to four times per keystroke** (`bench/keystroke.rs`). Rules are matched on
  that path and must be compiled once, not parsed per key.

## What this does not do

- **No repair slot for plugins.** The correction after the line stays the model's.
- **No streaming.** One answer per request; a token-by-token ghost needs the drawing path to accept
  partial answers.
- **No sandbox.** The trust gate still decides whether you run somebody's code. What is new is that
  the doctor will *say* which plugins can see your typing.
- **No `tag-order`.** zsh can order *within* a menu by tag; oslo's equivalent would be grouping the
  dropdown by kind, which is a presentation change and a separate piece of work.

## Sources

zsh-autosuggestions `ZSH_AUTOSUGGEST_STRATEGY` / `USE_ASYNC` / `BUFFER_MAX_SIZE` / `*_IGNORE`; zsh
compsys `completer`, `tag-order`, `group-order` and the `:completion:…` context; VS Code / Monaco
`InlineCompletionsProvider.groupId` and `yieldsToGroupIds`; nvim-cmp `group_index` and `priority`;
blink.cmp per-provider `enabled` / `async` / `timeout_ms` / `min_keyword_length` / `max_items` /
`score_offset` / `fallbacks` / `transform_items`; Emacs `completion-at-point-functions` and
`cape-capf-super`.
