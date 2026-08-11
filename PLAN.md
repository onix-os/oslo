# scratch

Named sessions that keep running when the terminal they were in goes away, modelled on
[scratch-rs](https://github.com/austinjones/scratch-rs). Behind a cargo feature, off by default, like `ssh`
and `vista`.

**One key is the whole interface.** `ctrl+\` opens the finder, and it does so in the same way
whether or not you are in a scratch. There is no `scratch` command, no subcommands and no arguments. A shell
never becomes a scratch on its own; nothing happens until the key is pressed.

## What it does

The whole feature is this table. Everything below serves it.

| where | you press | what happens |
|---|---|---|
| outside a scratch | `ctrl+\` | the finder: every scratch, plus a `create <query>` row |
| outside, in finder | Esc | nothing — back to your prompt, line intact, no scratch made |
| outside, in finder | pick one | attach to it |
| outside, in finder | type a new name, Enter | create it, attach |
| inside a scratch | `ctrl+\` | the same finder, **this scratch listed too** |
| inside, in finder | Esc | detach — back to the shell you pressed the key in, scratch still running |
| inside, in finder | pick the same one | straight back in, nothing changed |
| inside, in finder | pick another | leave this one running, attach to that |
| inside, in finder | type a new name, Enter | create and attach; the old scratch keeps running |

Nothing is ever killed by navigating. `$SCRATCH` holds the current name, for the prompt.

## The key

`ctrl+\` by default, `oslo.scratch.key`. There is no two-key sequence — Esc in the finder is the
disconnect, so one binding covers everything.

**It is handled in two different places, and they are not interchangeable.** Outside a scratch the shell
owns the terminal, so this is an editor keybinding like any other, fired at the prompt. Inside a scratch
the client owns it and the shell is on the far side of a pty, so it is a byte the client takes out
of the stream before forwarding. Same key, same finder, two mechanisms.

**The editor cannot decode it yet.** `term/input.rs:120` turns `0x01..=0x1a` into `Key::Ctrl`, and
`0x1c` falls past it to `text_key`, which has no character to make of a control byte and returns
`Ignored` — so a legacy `^\` does nothing at all, and nothing says why. The Kitty path is fine: `keyboard.rs` already decodes `\x1b[92;5u` to `Key::Ctrl('\\')`,
which `native.rs:58` names `ctrl-\`, the string a config binds. Since oslo pushes `\x1b[>5u`, most
terminals take the working path and the gap only shows on one that refuses Kitty — which is exactly
the kind of silent degradation worth closing first.

Extend `input.rs` to `0x1c..=0x1f` → `\ ] ^ _`, the same four keys and the same fix hexe needed
(`8c6a9f1` there). This is step 1, before anything else depends on the key arriving.

**It must be matched in two encodings.** The shell inside a scratch asks the terminal for the Kitty
keyboard protocol (`\x1b[>5u`, so its line editor can tell `^I` from Scratch). That request is output, so
it travels out through the client to the real terminal, and from then on the chord arrives as
`\x1b[92;5u` rather than as `0x1c`. A client watching for one byte works until the first prompt is
drawn and then never fires again.

`git show 04a72c3:crates/oslo-shell/src/pads/detach.rs` is that matcher, with tests. It is correct
and independent of the UX that commit belonged to — recover it rather than rewriting it.

Whatever the key is, it is swallowed from every program running inside a scratch. That is the cost, and
it rules out `^X` (nano's Exit, emacs' prefix).

## Two backends, one setting

```lua
oslo.scratch.daemon = false   -- default: a keeper per scratch
oslo.scratch.daemon = true    -- a registry process, as scratch-rs does it
```

Both present the same behaviour; the table above does not change between them.

|  | `daemon = false` | `daemon = true` |
|---|---|---|
| processes | one keeper per scratch | one daemon + one pty process per scratch |
| registry | the runtime directory | the daemon |
| when no scratches exist | nothing running | nothing running |
| transport | unix socket per scratch | one unix socket |

**Unix socket for the daemon too**, not scratch-rs's localhost TCP port and auth token. On Linux the
filesystem already answers the question the token exists to answer, and a port is reachable by every
process on the machine.

One trait, two implementations, chosen once at startup:

```rust
trait Scratches {
    fn list(&self) -> io::Result<Vec<Entry>>;
    fn create(&self, name: &str) -> io::Result<()>;
    fn connect(&self, name: &str) -> io::Result<UnixStream>;
    fn close(&self, name: &str) -> io::Result<()>;
}
```

```text
  daemon = false                        daemon = true

  scratch ──unix socket──► keeper           scratch ──unix socket──► daemon
                         │ pty                                 │
                         ▼                                     ├─► pty proc ─► oslo
                       oslo                                    └─► pty proc ─► oslo

  list = readdir                        list = ask the daemon
  alive = flock probe                   alive = the daemon knows
```

## Where the code goes

`crates/oslo-shell/src/scratch/`, feature-gated at the module. `dir.rs`, `store.rs`, `keeper.rs`,
`client.rs`, `detach.rs`, `wire.rs`, `daemon.rs`. The editor binding that opens the finder from a
prompt sits with oslo's other bindings, not here.

`dir.rs`, `store.rs` and `wire.rs` in `04a72c3` were sound and tested — the runtime directory with
its `O_NOFOLLOW|O_DIRECTORY` open and `fstat` on the fd, the meta files, the flock liveness probe,
the framed input / raw output split. Recover them. What was wrong there was the UX and the startup
wrapping, not the plumbing.

The runtime directory is `$OSLO_SCRATCH_DIR`, else `/tmp/oslo-$UID/scratch`, mode 0700, ownership checked on
the descriptor rather than the path.

## The finder is oslo's own widget

`ask::filter` (`crates/oslo-ui/src/ask/choose.rs:66`) is already a list with a typed query, fuzzy
matching and `Answer::Cancelled` on Esc. The one gap is the `create <query>` row.

Add `ask::choose::pick_or_create`, sharing `filter`'s internals:

```rust
pub enum Pick { Chosen(String), New(String) }
pub fn pick_or_create(spec: &Choice) -> Answer<Pick>
```

The row appears only when the query matches no existing entry, exactly as scratch-rs does it
(`create_tab_entry`, `fuzzy.rs:418`) — it is a row, not a second prompt. Existing `choose`/`filter`
callers are untouched.

**No hand-rolled prompt drawn on the alternate screen.** The finder is the shared widget, so it
carries the theme, the chrome and the key handling everything else in oslo has.

## Steps

Each ends with `make verify` green and is its own commit. Branch `feat/scratch` off `develop`.

1. `term/input.rs`: decode `0x1c..=0x1f` as control chords. Stands alone, fixes a real gap whatever
   happens to the rest, and everything below assumes the key arrives.
2. Cargo feature `scratch`; `oslo.scratch` settings (`key`, `daemon`, `dir`, `log_bytes`). A config carrying
   them is accepted by a build without the feature and does nothing.
3. `dir.rs` + `store.rs` recovered: runtime dir, paths, meta, flock liveness, list, sweep.
4. `keeper.rs`: pty, fork, socket, output log and replay.
5. `client.rs` + `detach.rs` recovered: raw termios, byte pump, both key encodings, `TIOCSWINSZ`.
6. `ask::pick_or_create`.
7. The key from the outside: the editor binding at a prompt, and the four outcomes of the top half
   of the table.
8. The key from the inside: the client's matcher, and the bottom half of the table.
9. The daemon backend behind `oslo.scratch.daemon`.
10. `docs/features/scratch.md`.

`$SCRATCH` is set in the process environment **and** in the shell's own variable table — setting only
the first leaves `$SCRATCH` empty in shell while `os.getenv` in the prompt can see it.

## Verification

- `make verify` green after every step; the differential corpus throughout.
- Startup unchanged, `hyperfine` n≥300 against a real config — nothing runs at startup, so this
  should be flat. Assert it rather than assume it.
- Keystroke latency through the pty, both backends.
- `kill -9` the client: the scratch survives and reattaches. `kill -9` the keeper: it is swept.
- A hostile runtime directory — wrong owner, 0755, a symlink — is refused.
- Binary size with and without the feature, recorded.
- `cargo build --all-features` and `cargo test --all-features` clean.

## Open

**Closing a scratch has no gesture yet.** The finder never kills anything, and there is no command, so
as it stands a scratch ends only when its shell exits. That is consistent — `exit` is how you close a
shell — but it leaves no way to reach a scratch whose shell is wedged. A key in the finder is the
obvious answer if one is wanted.

## Deferred

Trailing-slash names and subtabs, shell completions, statusline integration.
