# Directory environments

`direnv`, built in, plus a third file type it does not have.

Read against direnv at `b00e451` (Go, ~5.6k LOC in `internal/cmd`, plus a 1.4k-line `stdlib.sh`).

## What direnv actually is, and why ours is smaller

Almost every structural decision in direnv follows from one fact: **it is an external binary talking
to a shell it did not write.** It cannot see the shell's environment, cannot change it, and cannot
keep state between invocations. So:

| direnv does | because | oslo needs |
|---|---|---|
| shells out to `bash` to run `.envrc` | it has no evaluator | nothing — oslo *is* the evaluator |
| serialises the env diff into `DIRENV_DIFF` as gzip+base64 | nowhere else to keep it | nothing — memory |
| emits shell code you `eval` (`direnv export bash`) | cannot mutate its parent | nothing — direct |
| installs a prompt hook per shell, nine of them | must be re-invoked | nothing — the `cd` path |
| `DIRENV_DIR`, `DIRENV_FILE`, `DIRENV_WATCHES` in your env | same reason | in-memory state |

Five of direnv's mechanisms are a workaround for not being the shell. Reproducing them here would be
cargo-culting. What must be reproduced exactly is the part that is *not* incidental: the security
model, the load/unload lifecycle, and the file-precedence rules.

## The security model, copied exactly

**A directory you `cd` into must not be able to run code.** This is the entire reason direnv has an
allow list, and it is not a formality — `git clone` and `cd` is a completely ordinary sequence, and
without this that sequence is arbitrary code execution.

direnv's design, which we take verbatim:

* An rc file is inert until explicitly allowed. Until then it is *not read*, and the shell says so.
* Allow is keyed by `sha256(absolute_path + "\n" + file_contents)` — `rc.go:343`. Because the
  contents are in the hash, **editing an allowed file revokes it.** That is the property that
  matters: allowing a file once does not allow whatever it becomes later.
* Deny is keyed by `sha256(absolute_path + "\n")` — path only, no contents — so a denial sticks
  across edits. Asymmetric on purpose, and correct: "I trust this text" versus "I do not trust this
  place".
* Deny is checked *before* allow (`rc.go:155`), so a denial cannot be overridden by content.
* The tokens are empty files in a state directory, named by the hash. No parsing, nothing to get
  wrong, and `direnv prune` drops the ones whose file is gone.

We keep all of it, including the asymmetry and the check order.

## Files, in precedence order

Found by walking **up** from the current directory to the root, nearest wins — direnv's rule.

1. **`.envrc`** — shell. direnv runs this under bash; oslo runs it under oslo, which is the whole
   point of building it in. Same language as everything else in the shell.
2. **`.env`** — plain `KEY=value`, no execution. direnv's `dotenv`.
3. **`.env.lua`** — *ours, not direnv's.* Lua, and the reason this feature is worth building rather
   than shelling out to the real direnv.

All three load if present, in that order, so `.env.lua` can override `.envrc`.

## `.env.lua`, and why it is the interesting one

An environment variable is a poor way to say "in this project, `ctrl-g` runs the test suite" or
"this prompt should be red because it is production". Those are shell facts, and until now a config
could only state them globally. `.env.lua` scopes them to a directory:

```lua
env.DATABASE_URL = "postgres://localhost/app_dev"
env.PATH = oslo.path_add("./node_modules/.bin")

oslo.keys["ctrl-t"] = function(line) return { text = "cargo test" } end
oslo.prompt.right = "⚠ production"
```

**Everything set here must be reversible**, because leaving the directory must restore what was
there. That is the same requirement direnv has for the environment, extended to the rest of the
shell's state. The implementation is the same shape for all of it: record the previous value at load
time, put it back at unload time.

## The lifecycle

State, held in memory rather than in the environment:

```
loaded: Option<{ path, hash, diff, watches, restore }>
```

On every directory change, and only on a directory change:

1. Find the nearest rc file for the new directory.
2. If it is the same file, unchanged (mtime of every watched file), do nothing at all. This is the
   common case — most `cd`s are within one project — and it must cost nothing.
3. If a different rc applies, or none: **unload** the current one by applying the reverse diff and
   the recorded restores.
4. If a new rc applies: check deny, then allow, then load it and record the diff.

Unload before load, always, so moving between two projects never leaves the first one's variables
behind. direnv gets this right by construction and it is the bug most hand-rolled versions have.

## What we are not building

* **`stdlib.sh`'s 1.4k lines of `layout_*` helpers** — `layout_python`, `layout_go`, `layout_ruby`,
  `layout_pipenv`, twenty more. These are recipes, not mechanism, and every one of them is a
  guess about somebody's toolchain that goes stale. `PATH_add`, `dotenv`, `source_env`, `source_up`
  and `watch_file` are the load-bearing ones; those we provide.
* **`use nix` / `use flake`** — a whole subsystem with its own cache. Nothing stops an `.envrc` from
  calling `nix` itself.
* **The nine shell hooks** — no foreign shell to hook.
* **`direnv export` / `direnv dump`** — the protocol exists to cross a process boundary we do not
  have.

## Commands

`direnv allow` / `deny` / `reload` / `status` / `prune` / `edit`, matching direnv's names and
aliases (`permit`, `grant`; `block`, `revoke`). `direnv allow` with no argument means the current
directory, as it does there.
