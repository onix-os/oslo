# slowpoke — the shape an LLM ghost has

A suggestion provider that cannot answer on the keystroke path, so it says `request` instead of
`answer` and calls `reply` when the work finishes.

```sh
oslo plugin install examples/plugins/slowpoke
```

Then type `git s`, stop, and wait a moment: the ghost appears on its own, without a keystroke to
provoke the repaint.

It is slow and **deterministic** on purpose — `sleep` plus a table rather than a model — so it
exercises the whole asynchronous path while still being something a test can assert on. Swap the
`oslo.spawn` for a call to a model and nothing else in the file changes.

The four settings worth reading are the ones a real one would want:

| | |
|---|---|
| `debounce_ms` | ten keystrokes in one word is one question, not ten |
| `on_late` | `fill` never changes what is drawn; `replace` puts it ahead of your history |
| `min_chars` | a model asked about `g` is being asked nothing |
| `enabled` | the predicate that says *not in this directory* |
