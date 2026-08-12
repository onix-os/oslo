# Suggestions you can plug into

Two things the shell offers as you type — the **ghost** past the cursor and the **dropdown** on Tab —
are closed. A config can reorder the ghost's four built-in sources and can replace the dropdown for
one named command; it cannot add to either. This opens both, in a shape that works for a plugin
answering from a local database *and* for one answering from a language model over a network.

**Work on a new branch off `develop`.**

## Why this, and why now

Two plugins nobody can write today:

- **tldr** — 3,000 pages of "here is what people actually do with this command". `git <Tab>` should
  offer them with descriptions. Half of this is already possible: `oslo.completion.spec` (landed last
  branch) takes exactly the shape tldr has. The half that is missing is *adding* to a command oslo
  already knows, and answering for commands generically rather than one name at a time.
- **an LLM ghost** — regenerating the suggestion as you type, the way an editor does. Nothing about
  this fits today: the ghost's sources are a Rust enum, and the call is synchronous on the keystroke
  path, where an answer that takes 300 ms is not an answer at all.

The second is the one that decides the design. **A provider that can be slow is the general case**;
a fast one is the easy special case of it.

## What is there now

### The ghost

`OsloHelper::suggest` (`crates/oslo-ui/src/lib.rs:177`) walks `oslo.suggest.sources` in order and
takes the first source that answers:

```rust
History | Completion | Path | Prediction     // settings::Source — a closed enum
```

Reached from `Assist::hint_text` → `frame::draw` (`edit/session/frame.rs:88`), synchronously, once
per frame — which is once per keystroke. Guarded by three rules worth keeping: end of line only,
`sh` mode only, and `feature::on(SUGGEST)`.

### The dropdown

`OsloHelper::candidates` gathers; `config_candidates` (`completion.rs:116`) asks
`oslo.completion.for_command[name]` and **replaces** oslo's own candidates for that command
(`completion.rs:187`). `spec::custom` holds config-declared specs. `DropdownMenu::select_interactive`
then owns the terminal and its own key loop.

Three limits, all real:

- per command *by name* — there is no "answer for anything" hook;
- **replaces**, so a plugin that completes `git` loses oslo's built-in git spec;
- carries **no kind**, so `oslo.completion.sources` filters every config candidate out
  (`completion.rs:135` says so).

`on-completion-start` / `-select` / `-cancel` exist and are `answers: false` — observers.

### The part that changes everything

**The editor already knows how to wait for an answer that has not arrived.** `frame::next_input`
(`edit/session/frame.rs:22`):

```rust
if !idle_hook && !crate::prompt::refreshing() { return keys.read_event(); }   // block, the default
…
const REFRESH_SLICE_MS: i32 = 15;
match keys.read_event_within(slice) { …
    Timeout => if crate::prompt::generation() != seen { return Some(InputEvent::PromptRefreshed); }
```

A counter (`prompt::generation`), an outstanding-work count (`prompt::refreshing`), a 15 ms poll
slice used *only* while something is outstanding, and a synthetic non-key event that makes the loop
redraw. That is precisely what an asynchronous ghost needs, working in the tree today for
asynchronous prompts.

`lua/api/external.rs` is the third precedent and states the constraint outright: *"a prompt is on the
critical path of every keystroke"*, answered with `timeout_ms` and an `async` mode that uses the
previous output immediately.

**So this plan is mostly generalisation, not invention.**

## The design

### 1. One channel for "an answer may still be coming"

`prompt::refreshing`/`generation` are prompt-specific. Lift the pair into
`crates/oslo-ui/src/pending.rs` — an outstanding count and a generation, with the prompt as its first
caller and the ghost and the dropdown as the next two. `next_input` waits in slices when *anything*
is outstanding and answers `InputEvent::Refreshed` when *any* generation moves.

About 60 lines. Everything below depends on it and nothing below is possible without it.

### 2. Ghost providers

A fifth source, so a plugin's suggestion takes its place in the priority order the user already
controls:

```lua
oslo.suggest.sources = { "history", "provider", "predict", "path" }
```

Two forms, because the two cases are genuinely different:

```lua
-- fast: answers now, on the keystroke path. A database lookup, a table, a regex.
oslo.suggest.provider { name = "tldr", answer = function(ctx) return "…" end }

-- slow: answers later. The prompt is not held; the line repaints when it lands.
oslo.suggest.provider {
  name = "llm", debounce_ms = 120, timeout_ms = 2000,
  request = function(ctx, reply) oslo.spawn{ "llm", ctx.line, on_exit = function(out) reply(out) end } end,
}
```

`ctx` is `{ line, cursor, cwd, mode, last_status }`. A provider returning `nil` declines and the next
source is asked, exactly as the built-in sources behave.

**The continuation invariant is not negotiable.** The ghost is drawn as trailing text and accepted
with Right, so an answer that is not a continuation of what is typed would be a lie about what the
key does. An answer that does not start with the line is **refused and reported once** through
`messages` — not silently trimmed, which would produce a suggestion the plugin never wrote. A plugin
that wants to *replace* the line is asking for the repair slot, which is a separate question left out
of this plan.

### 3. Completion providers

Additive, any command or one, and carrying a kind:

```lua
oslo.completion.provider {
  name = "tldr",
  kind = "example",                  -- shown as the badge, and addable to oslo.completion.sources
  answer = function(ctx)             -- ctx = { command, words, current, cwd }
    return { { display = "git commit --amend", description = "change the last commit" } }
  end,
}
```

- **Adds** rather than replaces. `for_command` keeps its meaning — *I own this command* — and stays
  the escape hatch for a plugin that wants oslo's own candidates gone.
- **A kind, declared once**, so `oslo.completion.sources` can name it and the badge column can show
  it. This also fixes the existing hole where `for_command` candidates carry none.
- Async by the same two-form shape as the ghost. A dropdown that is already open gains rows when a
  slow provider answers; `select_interactive` has its own key loop and already polls
  (`finder/run.rs:92` does the same trick for the scanner animation).

### 4. Staleness, debounce, deadline

The three ways this feature is normally got wrong.

- **Staleness.** Every request carries the generation it was made at. An answer whose generation has
  moved is dropped, never drawn. Without this, typing `gi`, then `t`, then ` ` shows the answer to
  `gi` under `git ` — the classic async-suggestion bug.
- **Debounce.** `debounce_ms` per provider: no request is made until the line has been still that
  long. A model asked on every keystroke is asked ten times for one word.
- **Deadline.** `timeout_ms` per provider, and a **sync** provider gets a budget too: one that
  overruns it repeatedly is disabled for the session and says so through `messages`. A shell that
  feels broken because somebody's plugin is slow must be able to say which plugin.

### 5. Trust, and the switch

**A ghost provider sees every keystroke** — including the ones you did not run and the ones you
retyped because you got a password wrong. An LLM provider ships them somewhere. That is a bigger
privacy surface than anything a plugin can reach today, and it must be:

- named in `oslo plugin doctor`, so "what can see my typing" has an answer;
- covered by `oslo.feature` (`SUGGEST` already exists) so it can be turned off mid-session;
- silent under the existing no-trace rules — a provider must not be asked at all when the line is
  secret, when `HISTFILE=""` set `no_trace`, or when the leading-space convention is in force. The
  veto work already established that every sink asks one flag; this is one more sink.

## Order

Each step ends with `make verify` green and is its own commit.

1. **`pending`** — lift the prompt's outstanding/generation pair into a shared module; `next_input`
   waits on any of them. No behaviour change; the prompt tests are the proof.
2. **Ghost providers, sync only.** The fifth source, the registry, the continuation invariant, the
   sync budget. Measurable against `bench/keystroke.rs` with no provider installed.
3. **Ghost providers, async.** `request`/`reply`, debounce, staleness by generation, deadline.
4. **Completion providers, sync.** Additive, kinds, `sources` integration, and the badge.
5. **Completion providers, async**, gaining rows into an open dropdown.
6. **Two worked examples**, both in `examples/plugins/`: `tldr` (sync, `oslo.db`-backed, generated
   specs plus example candidates) and `echoer` (async, `oslo.spawn`-backed, standing in for an LLM
   with something that answers slowly and deterministically — so it can be a test).

Steps 1–5 are core and must work in `oslo-minimal`; nothing here is behind a cargo feature, because
the ghost and the dropdown are not.

## Verification

- `bench/keystroke.rs` **before and after step 2**, min-of-N: a shell with no provider installed must
  not pay for the mechanism. If it does, the registry lookup moves behind a "any providers at all"
  atomic.
- **A test that a stale answer is never drawn.** Type, request, type again, answer the first request
  — the frame must show the second line's suggestion or none, never the first's.
- **A test that a non-continuation is refused**, and reported exactly once rather than per keystroke.
- **A test that a slow sync provider is disabled** rather than being paid for on every key.
- The pty tests are where the async paths belong: `tests/terminal_semantics/` already drives a real
  editor, and an async ghost that repaints is only observable there.
- `oslo-minimal` builds and its ghost still works with no providers registered.

## Things that will bite

- **`edit/session.rs` is 595 lines and `completion.rs` is 588** — both cross the 600-line limit on the
  first step that touches them. Split by subject before adding, not after.
- **Reentrancy.** `config_candidates` already documents it (`completion.rs:121`): the hook runs Lua,
  Lua can complete another word, and the outstanding `RefCell` borrow panics. Every new registry has
  the same hazard and needs the same clone-before-call.
- **Lua is not `Send`.** A provider's `answer` runs on the shell's thread; only the *work* an async
  provider starts may leave it, which is why `request`/`reply` is shaped like `oslo.spawn` rather
  than like a promise. `reply` is delivered at a safe point, not from the worker thread.
- **`InputEvent::PromptRefreshed` is named for the prompt.** Renaming it touches the editor tests.
- **The dropdown owns the terminal.** Rows arriving while it is open must not move the selection —
  a row inserted above where your cursor is means Enter runs something you did not choose.

## What this does not do

- **No repair slot for plugins.** The correction after the line stays the model's.
- **No replacing the ghost's built-in sources** — a provider joins the order, it does not displace
  `history`, whose whole value is that it only ever offers something you really ran.
- **No sandbox.** Unchanged: the trust gate decides whether you run somebody's code. What is new is
  that the doctor will *say* which plugins can see your typing.
- **No streaming.** One answer per request. A token-by-token ghost is a different feature and would
  need the drawing path to accept partial answers.

## Open, and worth deciding before step 3

**What an async ghost does about the model.** With `vista` installed, `predict` answers in ~4 µs and
an LLM answers in ~300 ms. If both are in `sources`, the model's answer is drawn first and then
replaced when the slow one lands — text under the cursor changing on its own, half a second after you
stopped typing. The alternatives are: never replace an answer already drawn (the slow provider only
ever fills a *gap*), or let it replace and accept the flicker. **I would take the first**, but it is
a decision about how the shell feels rather than about what is correct, and it should be made
deliberately rather than discovered.
