# Secrets

A value kept encrypted on disk and handed out when something asks for it — with the key deliberately
somewhere else, so the store can go where files go.

```sh
oslo secret set stripe                 # asks for the value, masked; or reads standard input
oslo secret get stripe                 # writes it to standard output, and nowhere else
oslo secret run TOKEN=stripe -- curl … # one value, one child, no shell in between
oslo secret list
oslo secret where                      # the two directories, and which of them may be committed
oslo secret rm stripe
```

## Two features, because they answer different questions

| | | |
|---|---:|---|
| **`secrets`** | +96 KB, 1 package | the *filing*: stores, names, `run`, the lazy variable, `oslo.secret`, the hooks. **No crypto of its own** |
| **`crypt`** | +60 KB, 17 packages | the *built-in mechanism*, so a fresh install encrypts without being told anything |

A distribution shipping `/bin/sh` can take the filing alone and name the machine's own tool —
`age`, `gpg`, `systemd-creds` — in one line of configuration. Someone who wants it self-contained
keeps `crypt` and never thinks about it. Both are off in `oslo-minimal`.

<!-- demo:begin -->
<!-- demo:end -->

## Why this is a shell's problem

Every secret a person keeps ends up in a shell eventually: exported for one command, pasted into a
`curl`, read out of a `.env` that should never have been written. The shell is where secrets are
*used*, and it is the one program in the chain that can hold a value in memory, hand it to a single
command, and never write it down.

Anything outside the shell has the opposite problem. A secrets manager that is not the shell has to
put the value somewhere the shell can read — an environment variable set at login, a file, a
`source`d fragment — and each of those is the plaintext this was supposed to avoid.

## A store is three things

A **directory**, an ordered list of **keys**, and the **mechanism** that seals and opens its files.

```
$XDG_DATA_HOME/oslo/secrets/NAME.sealed       the `user` store
$XDG_DATA_HOME/oslo/stores/work/              a named store
$XDG_DATA_HOME/oslo/plugins/notes.secrets/    a plugin's own, beside the .kv oslo.db makes for it
$XDG_STATE_HOME/oslo/key                      the key
$XDG_STATE_HOME/oslo/secrets.conf             which stores exist, and what each is made of
```

The mechanism is the one that decides **where a store can be used**, so it is worth its own table:

| | does the crypto | reachable from |
|---|---|---|
| default | oslo's own, in this process | anywhere the binary runs |
| `encrypt`/`decrypt command` | another program, down a pipe | anywhere: a subprocess, not a shell |
| `crypto hook` | a Lua handler | only a shell that has read your config |

## The built-in mechanism

**A sealed box: a key you keep, recipients you publish.** One random *file key* encrypts the value,
and that file key is wrapped once per recipient against their public half, each wrap using a fresh
ephemeral X25519 keypair.

```
OSLO2 │ n │ n × [ ephemeral public (32) │ wrapped file key (48) ] │ nonce (24) │ ciphertext ‖ tag
```

**The secret half is the profile's, not a file of its own.** A store derives its key from the
[profile key](profiles-and-histories.md#why-a-key-and-not-just-the-name) — HKDF over the profile's
key, salted with the store's name — so a machine keeps one secret rather than two, and
`oslo profile export` carries the history *and* the secrets in one step. Two stores under one
profile still have different keys, so a file from one cannot be opened with the other's.

`$OSLO_SECRET_IDENTITY` still wins when it is set, and `key file`/`key command` still override per
store: naming a file is somebody saying *this one*, and that has to keep meaning what it says.

The two halves look different on purpose, because one of them is safe to publish and the other is
not:

```
OSLO-KEY-1:hqFoSGbhpF797BbBSjB+95yipkKjh2g2kEaXOFPlIYk=    the file, mode 0600
OSLO-PUB-1:zyzEW4hwVZdjajit/OwLB+o2647dl8YfoWY+mdCS+QM=    what you hand out
```

Pasting one where the other belongs is refused rather than quietly accepted — a published half read
as a key would make the store openable by everybody who has it.

That indirection is what a single symmetric key cannot have: **a store can be readable by several
keys without any of them being shared.**

```sh
oslo secret key init                      # on the other machine: prints its OSLO-PUB-1 half
oslo secret recipient add OSLO-PUB-1:…    # here
oslo secret rotate                        # so what already exists reaches them too
```

Your laptop keeps one secret, the server keeps another, and the store lists two recipients. Nobody
hands anybody a private key, and taking a machine off the list is one `recipient rm` and a `rotate`.
**Adding a recipient does not re-encrypt anything** — they can read what is written after today, and
`rotate` is the separate, deliberate step, because *who could read this before I changed it* has a
permanent answer.

`X`ChaCha for the payload, so the 192-bit nonce can be drawn at random on every write with nobody
counting. The wrapping key comes from HKDF-SHA256 over the shared curve point, salted with both
public halves: an X25519 output is a curve point rather than a uniform key, and the salt binds a
wrap to the pair it came from. Everything random — nonces, file keys, ephemeral secrets — comes from
`getrandom`, never a seeded generator.

Two consequences of it being an AEAD: a file that has been edited is refused rather than decrypted
into rubbish — including an edit to the recipient list — and the same value written twice produces
two different files, so nobody holding your backups can tell which secrets changed between them.

### What it deliberately is not

**Not the age format.** `age -d` will not open these files. What was worth having was the key model,
and carrying the format meant bech32, scrypt, a localised error catalogue and thirty-two packages —
against seventeen for the model alone.

No passphrase, and no key derivation from one: the secret is a file at mode `0600`, like
`~/.ssh/id_ed25519`.

**And none of it is compulsory.** The mechanism is one of three, and the other two do not involve
any of this:

```sh
oslo secret --store team cipher encrypt -- age -R ~/.config/age/recipients.txt
oslo secret --store team cipher decrypt -- age -d -i ~/.config/age/identity
```

That is the door to real age files, to `gpg`, to `systemd-creds --with-key=tpm2`, and to a YubiKey
through `age-plugin-yubikey`. A store that names one never reaches the code above.

## Configuration is a file, not Lua

`oslo secret get` is dispatched as a tool: it never builds an `Environment`, never reads
`config.lua`, never starts a Lua interpreter. That is not an oversight. `$(oslo secret get
gh-token)` has to work from `dash`, from `cron`, from a `Makefile` and from a container, none of
which have run an oslo config — so configuration that only existed after `config.lua` had run would
apply in your interactive shell and silently not apply anywhere else.

So it is a flat file that the process doing the decrypting reads for itself:

```
# ~/.local/state/oslo/secrets.conf
default work

[work]
directory /home/you/src/dotfiles/secrets     # a store meant to be committed
key file /home/you/.ssh/oslo-work
key command pass show oslo/key
recipient OSLO-PUB-1:jmtSV18HQJ/Ph1RXFGFTX8vbjNYpveSlwm1n9AwJySY=   # you
recipient OSLO-PUB-1:zyzEW4hwVZdjajit/OwLB+o2647dl8YfoWY+mdCS+QM=   # the build server

[team]
directory /home/you/src/team/secrets
encrypt command age -R /home/you/.config/age/recipients.txt
decrypt command age -d -i /home/you/.config/age/identity
```

**Beside the key, not inside the store.** The store is meant to be committable; this file names this
machine's key paths, and a `secrets.conf` restored from somebody else's backup silently changing
where this machine looks for a key is not a thing to ship.

**A malformed line is an error, never a skipped line.** In a build without `crypt` a `key` line is
refused outright rather than parsed and ignored — the key belongs to whatever program that store
names, and a line that quietly did nothing is how somebody comes to believe a store is protected by
something it is not.

Nothing needs to be edited by hand:

```sh
oslo profile key init                      # the key a store derives its own from
oslo secret recipient                      # the half to publish, derived and shown
oslo secret key add file ~/.ssh/oslo-work  # or override it, per store
oslo secret key add command -- pass show oslo/key
oslo secret key list

oslo secret recipient add OSLO-PUB-1:…     # a colleague, or your other machine
oslo secret recipient add --from RECIPIENTS
oslo secret recipient --export > RECIPIENTS
oslo secret recipient rm OSLO-PUB-1:…
oslo secret rotate                         # re-encrypt everything to the list as it now stands

oslo secret cipher encrypt -- age -R ~/.config/age/recipients.txt   # or hand it all to age
oslo secret cipher decrypt -- age -d -i ~/.config/age/identity
```

The commands splice lines rather than re-render the file, so comments and ordering survive.

### Which store an invocation means

| | scope |
|---|---|
| `--store NAME` | this invocation |
| `$OSLO_SECRET_STORE` | this shell, inherited by every child — `OSLO_SECRET_STORE=work make deploy` |
| `default NAME` in `secrets.conf` | this machine |
| `user` | built in |

## Keys, including ones oslo cannot compute

A key is a **file** to read, or a **command** to run whose output is the key. The second exists
because the alternative is compiling every way a person might hold a key into a shell meant to be
`/bin/sh` — a password manager, a smartcard wrapper, a decryption service:

```
key command pass show oslo/key
key command gpg --quiet --decrypt /home/you/oslo-key.gpg
```

**File keys are tried before program keys, always.** A store that opens with a file never runs the
program another key source names — no `$PATH` walk, no fork, and a cron job on a machine that cannot
reach the other key degrades instead of hanging on it.

What fences the command:

* **argv, never a shell string.** Nothing reaches `/bin/sh`, so there is no quoting layer to get
  wrong and no `$(…)` in a configuration file.
* **Never in a `plugin.*` store.** A plugin's store cannot fork, whichever door the line came
  through — the command refuses to write it, and a hand-edited `secrets.conf` is refused when the
  store is opened.
* **`$OSLO_SECRET_NO_EXEC`**, set to anything non-empty, skips every command source and names it in
  the failure. Exported once by a cron job or a container and inherited by every child, it makes
  *this will not fork* something to assert rather than infer.

A key file under a `.git` is one `git add -A` from being published, and the person that happens to
did not choose it — they moved a directory a year later. Every `oslo secret` says so:

```
oslo secret: the key is inside the git repository at /home/you
oslo secret: move it with $OSLO_SECRET_IDENTITY, or the next commit publishes it
```

**That check was measured, because the first version cried wolf.** `~/.git` on the machine it was
written on is an empty directory left behind by something, and `git -C ~ rev-parse` calls it *not a
git repository*. A real one is a directory with `HEAD` in it, or a *file* saying where the directory
is, which is what a worktree and a submodule have.

## What happens on a get

```
oslo secret get stripe
   │
   ├─ read secrets.conf                which store, which keys, which mechanism
   ├─ path("stripe")                   a name is a filename: no `/`, no `..`, no leading `.`
   │                                   — refused, not sanitised
   ├─ read secrets/stripe.sealed
   ├─ file keys, in the order written
   │     └─ none opened it? ─► program keys    only then is anything run
   ▼
XChaCha20-Poly1305 ─► the value on stdout, with nothing added
```

Writing goes the other way, and every file — the key, the ciphertext, `secrets.conf` — is written to
a scratch file that is already mode `0600` and then renamed. There is no instant at which any of
them exists and is readable by somebody else, and no half-written file if the machine stops.

**A trailing newline is dropped on `set`.** The value came from a line somebody typed or from a
`printf` in a script, and a token with `\n` on the end fails authentication in a way that takes an
hour to find.

**At a terminal, `set` asks rather than reads.** Standard input there is the keyboard, so reading it
to end of file means the value is typed in the clear, into the scrollback, and finished with a
Ctrl-D nobody is told about. Instead it is the shell's own masked [`ui input`](userin.md).

## The two ways a value reaches a program

```sh
export GITHUB_TOKEN=$(oslo secret get gh-token)      # in this shell, and every child, for good
oslo secret run GITHUB_TOKEN=gh-token -- gh pr list  # in one child, and nowhere else
```

A command substitution puts the value in the *calling shell*, where a `set` prints it and
`/proc/PID/environ` holds it for as long as that shell lives. `run` execs the command directly with
one extra variable and no shell in between. `VAR=` with no name means the secret named after the
variable, lowercased and hyphenated: `oslo secret run GH_TOKEN= -- gh` reads `gh-token`.

## The half that makes it a shell feature

A store you have to remember to call is a store you will paste out of. The other half is a stored
[variable](macros.md#a-variable-holds-a-recipe-not-a-value) whose body is the *recipe*:

```sh
oslo macros add --var 'GITHUB_TOKEN=$(oslo secret get gh-token)'
```

Nothing has run yet. The first time something in a shell reads `$GITHUB_TOKEN`, that line is
evaluated — once, in that shell — and from then on it is an ordinary exported variable. A shell that
never mentions the name never decrypts anything. The same line as an `export` in `config.lua`
decrypts at every shell start, on every machine, for ever, whether or not anything wanted it.

## From Lua

```lua
oslo.secret.get("gh-token")            -- the store this shell would use
oslo.secret.set("gh-token", value)
oslo.secret.list()   oslo.secret.stores()

local work = oslo.secret.open("work")  -- another store, by name
work:get("deploy")  work:set("deploy", v)  work:forget("deploy")  work:list()  work:where()

local sealed = oslo.secret.seal("anything")   -- the crypto without the filing
oslo.secret.unseal(sealed)
```

`seal` and `unseal` are base64 of a whole sealed file, so a plugin can encrypt something it keeps
somewhere else of its own. Base64 because a sealed file is binary and a Lua string is UTF-8.

### What a plugin may reach

A plugin gets **its own encrypted store**, unconditionally, with no name to pass because the name is
not the plugin's to write:

```lua
local mine = oslo.secret.mine()
mine:set("cursor", "42")
```

It lives at `$XDG_DATA_HOME/oslo/plugins/<name>.secrets/`, beside the `.kv` that
[`oslo.db`](plugins.md#the-database) already makes for it, so uninstalling stays an `rm -r`. It is
sealed with *your* key rather than one of its own: a per-plugin key would enforce nothing (every
plugin can read every file through `oslo.fs`) while adding private keys at rest and making a lost
one a data-loss event the machine's owner cannot recover from.

**`plugin.` is refused from Lua, always, to every caller**, so `oslo.secret.mine()` is the only door
to a plugin store. The check is on the *name asked for* rather than on who is asking, which is what
makes it hold: it cannot be sidestepped by deferring the call into a hook or a timer. The command
line has no such rule — `oslo secret --store plugin.notes list` works, because you must be able to
see and remove what a plugin kept on your machine.

For **your** secrets, a plugin declares what it will read:

```lua
-- plugin.lua
return {
  name = "gh", version = "0.3.1",
  builtins = { "gh-pr" },
  secrets = { "gh-token", "gh-host" },     -- names, never a wildcard or a prefix
}
```

`oslo plugin install` prints it before you decide to trust it, and it can, because the manifest is
evaluated in a fresh interpreter with no `oslo` global:

```
gh 0.3.1 reserves: gh-pr
  secrets: gh-token, gh-host   it will be able to read these
install and allow it to run? [y/N]
```

**This is a disclosure, not a sandbox.** A plugin can run `oslo secret get` as a subprocess and
nothing here stops it; oslo [declines to sandbox plugins by name](plugins.md#trust). What the
declaration buys is a plugin that can be *caught contradicting its own manifest*. It also only holds
while the plugin's file is loading: a handle stashed in a global outlives the load, and a call from
a hook is indistinguishable from one made at the prompt.

## Pluggable: hooks do the crypto, Lua does the storage

Two independent axes, either or both replaceable:

| axis | replaced by | declared in |
|---|---|---|
| **crypto** — what the bytes are | `on-secret-encrypt` / `on-secret-decrypt` | `crypto hook` in `secrets.conf` |
| **storage** — where the bytes live | `oslo.secret.define` | Lua, at load |

```lua
oslo.on["on-secret-encrypt"](function(store, name, sealed)
  if store ~= "vault" then return nil end        -- nil is "not mine"
  return my_own_encryption(sealed)
end)

oslo.secret.define("vault", {
  get    = function(name) return db:get(name) end,
  set    = function(name, sealed) db:set(name, sealed) return true end,
  list   = function() return db:keys() end,
  forget = function(name) db:delete(name) return true end,   -- optional
})
```

Everything crossing to Lua is **base64**, both hooks and both storage handlers.

### Calling `age` — and so a YubiKey — from a hook

A handler is handed base64 and must answer base64, while `age` speaks raw bytes on a pipe. A Lua
string cannot carry those, and putting the payload in argv would publish the plaintext to `ps`, so
this is the one thing a hook cannot build for itself. `oslo.secret.through` is the pipe:

```lua
local recipients = os.getenv("HOME") .. "/.config/age/recipients.txt"   -- has age1yubikey1…
local identity   = os.getenv("HOME") .. "/.config/age/yubikey.txt"

oslo.on["on-secret-encrypt"](function(store, name, sealed)
  if store ~= "yubi" then return nil end
  return oslo.secret.through({ "age", "-R", recipients }, sealed)
end)

oslo.on["on-secret-decrypt"](function(store, name, sealed)
  if store ~= "yubi" then return nil end
  return oslo.secret.through({ "age", "--decrypt", "--identity", identity }, sealed)
end)
```

**Standard error is inherited**, so the plugin's *touch your key* prompt reaches your terminal —
captured, it would not, and the shell would appear to hang for reasons nobody could see.

**A hook is the right route when you want logic** — different keys per secret, a notification before
the touch, a fallback. **When you only want a program run, use `cipher command`**: it is a
subprocess rather than a hook, so it also works from `cron`, from `dash`, and behind
`$(oslo secret get …)` in a macro variable — none of which run Lua.

### The rule that keeps it honest

**The file declares which mechanism a store uses; Lua supplies the mechanism.**

A hook only exists in a process that ran your config. So `crypto hook` is written in `secrets.conf`,
and a process with no Lua that meets it says so:

```
oslo secret: vault: its crypto is `crypto hook`, and nothing is attached to on-secret-decrypt
here. A hook needs a shell that has read your config; `oslo secret` from a script, a Makefile
or cron never does
```

That check runs *before* anything touches a file, so the reason you get is the real one rather than
"no such file". Declaring `crypto hook` and an `encrypt command` in one store is refused outright —
they have different reach, and quietly picking one is how a store becomes readable in one process
and not another.

### `nil` means "not mine"

A handler that returns `nil` declines and the next one is asked. So several plugins can each serve
their own store off one hook, and a store nobody claims is a **refusal** rather than a fall back to
the built-in mechanism — falling back would seal it with a key the store was never meant to use.

### Watching, without seeing

`pre-secret-access` and `post-secret-access` fire on every read and write, on the native path and the
defined one. They are told the store, the name, and `"read"` or `"write"` — and **never the value**,
because a hook that logs is the likeliest thing anybody writes on them and a log of secrets is worse
than no log.

### What may not be replaced

`oslo.secret.define` refuses `user` and anything under `plugin.`. The first is what the command line
means, and shadowing it would make `oslo.secret.get` and `oslo secret get` disagree about your own
secrets; the second is reached only through `oslo.secret.mine()`.

## Configuration

| | |
|---|---|
| `$XDG_DATA_HOME` | where stores live, under `oslo/`. Falls back to `~/.local/share` |
| `$XDG_STATE_HOME` | where the key and `secrets.conf` live. Falls back to `~/.local/state` |
| `$OSLO_SECRET_IDENTITY` | a key file for every store, absolutely — checked first, and beats the profile |
| `$OSLO_SECRET_CONF` | the configuration file, absolutely |
| `$OSLO_SECRET_STORE` | which store this shell and its children mean |
| `$OSLO_SECRET_NO_EXEC` | skip every mechanism that would run a program |

| hook | |
|---|---|
| `on-secret-encrypt` | asked to seal, for a `crypto hook` store. Answers base64, or `nil` for "not mine" |
| `on-secret-decrypt` | the same, the other way |
| `pre-secret-access` | a secret is about to be read or written: store, name, `read`/`write` |
| `post-secret-access` | it just was |
| `oslo.secret.through(argv, base64)` | not a hook: the binary-safe pipe a handler calls to reach another program |

There is no `config.lua` for any of this, and that is the point of the section above: a path read
after Lua has run is a path a `cron` job never sees.

## Measurements

What each half costs, measured on the real binary at the release profile:

```text
  every other feature on   6,640,032 bytes   78 packages
  + secrets (the filing)   6,746,528 bytes   79 packages    +104 KB,  +1 package
  + crypt (the mechanism)  6,807,968 bytes   96 packages    + 60 KB, +17 packages
```

**This used to be `age`, at +160 KB and +32 packages.** What age brought was the file format on top
of the same key model — bech32, scrypt, a localised error catalogue, the recipient types for
hardware and post-quantum. Dropping the format and keeping the model costs 60 KB: `x25519-dalek`
and the curve beneath it, `chacha20poly1305`, and `hkdf` against the `sha2` already here.

An earlier version of `crypt` was one symmetric key at 36 KB and 10 packages. It was 24 KB cheaper
and could not let two machines read one store without copying a private key between them, which is
most of what a secrets store is for.

What a read costs, interleaved, min of five runs of three hundred, against the same binary doing a
`secret list` — same process, same directory, no key opened and nothing decrypted:

| | |
|---|---|
| `oslo secret list` | 598 µs |
| `oslo secret get` | **807 µs** |

So reading the key, the X25519 exchange, the HKDF and opening the payload come to about **209 µs**,
and the rest is starting a process. The exchange is what the symmetric version did not pay; it is
also what lets somebody else read the file.
Delegating to a program instead costs a spawn: `systemd-creds` measured **45,254 µs** on this
machine, which is the price of the flexibility and the reason the built-in mechanism is not simply
deleted.

## What it cannot do

- **Recipients are oslo's own, not age's.** A colleague who already has an age identity cannot be
  added; they publish an `OSLO-PUB-1` half or the store hands its crypto to `age`. There is no
  interoperability with the age ecosystem by design.
- **No passphrase, and no key derivation.** The key is protected by the filesystem, mode `0600`,
  like `~/.ssh/id_ed25519`. A passphrase asked on every read is what teaches people to keep the
  value somewhere else instead.
- **A hook-backed store is unreachable outside a shell that ran your config** — no `cron`, no
  `dash`, no `$(oslo secret get …)` in a macro variable. Stated in the file rather than discovered,
  but a real limit: for anything a script must read, use the built-in mechanism or a command.
- **Nothing here is protected from another plugin.** All Lua runs on one interpreter with one `oslo`
  global, and `oslo.fs` reads any file this user can. A plugin's store is encrypted against the
  *disk*; it is not isolated from other plugins.
- **The declaration is not enforcement.** `oslo.proc.exec("oslo", "secret", "get", …)` walks past it.
- **No key rotation, and no revocation of what is already out.** `rotate` re-encrypts to the list as
  it now stands, so removing a recipient stops them reading anything written *afterwards* — it
  cannot reach a copy of the store they already have. Deleting a key makes everything sealed only to
  it unreadable.
- **At most 255 recipients**, which is one byte in the header and more than anybody has.
- **A value is not hidden from the process that asked for it.** If you export it, every child gets
  it. `run` is the narrower door, and it is still an environment.
- **No `git`-aware anything.** The repository check is one warning about where the *key* is.

## Where it lives

| | |
|---|---|
| `crates/oslo-base/src/secrets.rs` | `Store`: paths, seal, unseal, rotate, and the user-store shorthands |
| `crates/oslo-base/src/secrets/native.rs` | the sealed box: the wrap, the key file, the format |
| `crates/oslo-base/src/secrets/recipient.rs` | who a store is written for, and what a build makes of one |
| `crates/oslo-base/src/secrets/conf.rs` | `secrets.conf`: parsed, and edited line-wise |
| `crates/oslo-base/src/secrets/key.rs` | `KeySource`: the profile, a file, or a program — and what fences the program |
| `crates/oslo-base/src/track/profile/key.rs` | the profile key every store derives from |
| `crates/oslo-base/src/secrets/crypto.rs` | the three mechanisms, and why only one can be unreachable |
| `crates/oslo-base/src/secrets/cipher.rs` | handing encryption and decryption to another program |
| `crates/oslo-base/src/secrets/hooked.rs` | asking Lua to do the crypto, and telling it what was touched |
| `crates/oslo-runtime/src/lua/api/secret.rs` | `oslo.secret`, and what a plugin's handle may reach |
| `crates/oslo-runtime/src/lua/api/secret/defined.rs` | `oslo.secret.define` — a store whose bytes are Lua's |
| `src/cli/secret.rs` | the command line, with `key`, `cipher` and `run` beside it |
