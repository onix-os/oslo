# Plan: Add a plain secret database with hook-managed encryption

> **Executor instructions**: Follow the phases in order. Run every verification gate before moving
> on. Do not implement adjacent refactors. If a STOP condition occurs, stop and report it instead of
> inventing another security model. Never use a real credential in a test, log, commit, issue,
> example, or terminal transcript.
>
> **Architecture is fixed**: `oslo secret` is an ordinary client for a separate plaintext Tagdata
> database. It has no encryption setting, provider, codec, pre-hook, post-hook, age integration, GPG
> integration, or key handling. Optional encryption is user configuration built from the existing
> general `oslo.on.pre_cmd` and `oslo.on.post_cmd` hooks in an imported `secrets.lua` module.
>
> **Drift check (run first)**:
>
> ```sh
> git diff --stat 31a39cc..HEAD -- \
>   crates/oslo-base/src \
>   crates/oslo-runtime/src/startup \
>   crates/oslo-runtime/src/lua \
>   crates/oslo-shell/src/exec \
>   crates/oslo-ui/src/settings \
>   crates/oslo-ui/src/ask/input.rs \
>   src/cli.rs src/cli src/main.rs \
>   tests docs/features
> ```
>
> If an in-scope file changed, compare the current-state evidence below with the live checkout before
> editing. STOP if a changed API invalidates a phase; do not force old line numbers onto new code.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `31a39cc`, 2026-08-11
- **Status**: TODO

## Outcome

After implementation Oslo has two independent layers:

1. `oslo secret` reads and writes a dedicated, ordinary Tagdata database containing named values.
   The tool always sees a normal file and works without any encryption software.
2. The interactive shell has a conservative privacy gate that prevents high-confidence
   secret-bearing command lines from entering Oslo-owned history, recall, tracking, terminal marks,
   notices, frecency, chain reports, or xtrace.

Users who want encryption create:

```text
$XDG_CONFIG_HOME/oslo/lua/secrets.lua
```

and import it from their normal config:

```lua
require("secrets")
```

That Lua module attaches to the existing general command hooks. An age-based module can lock the
vault, decrypt `secrets.db.age` to the ordinary `secrets.db` before a foreground `oslo secret`
command, then encrypt it again, remove the plaintext file, and release the lock after the command.
Replacing age with GPG, an SSH-capable age identity, a custom program, or no wrapping at all does
not change one line of the secret tool.

## Non-negotiable architecture

### `oslo secret` is a normal database tool

The Rust implementation owns only:

- the default plaintext database path;
- private directory and file permissions;
- a small schema and ordinary CRUD operations;
- masked value entry and exact value output;
- CLI parsing, help, diagnostics, and exit statuses;
- automatic command-line privacy detection in the interactive shell.

It does **not** own:

- encryption or decryption;
- keys, recipients, identities, agents, passphrases, or hardware tokens;
- `age`, `gpg`, `pass`, Secret Service, Vault, or SOPS adapters;
- an `oslo.secret` Lua settings table;
- secret-specific pre/post hook types;
- a daemon;
- a second executable or a builtin called `secret`.

There is one canonical entry point:

```text
oslo secret ...
```

Do not add a bare `secret` builtin. The imported general hook module matches the parsed `oslo
secret` command before the external Oslo tool process opens its ordinary database.

### Encryption is general hook configuration

Encryption behavior lives entirely in user-owned Lua:

```lua
oslo.on.pre_cmd(function(command) ... end)
oslo.on.post_cmd(function(command) ... end)
```

Do not add any of these:

```lua
oslo.secret.pre_hook = ...
oslo.secret.post_hook = ...
oslo.secret.encryption = ...
oslo.secret.codec = ...
oslo.secret.provider = ...
```

The repository supplies and tests an age-oriented reference module at
`docs/features/secrets.lua`. A user copies or symlinks that file to
`$XDG_CONFIG_HOME/oslo/lua/secrets.lua`, edits its ordinary Lua constants, and writes only
`require("secrets")` in `config.lua`. The module is documentation/configuration, not Rust source and
is not embedded in the binary.

### No encryption is a supported choice

Without `require("secrets")`, the database remains a mode-`0600` plaintext Tagdata file. Oslo must
not warn repeatedly, refuse to work, or silently select an encryption tool. The user chose the
at-rest policy by choosing which general hooks to install.

## Product contract

### Default paths

The plaintext database path is:

```text
$XDG_DATA_HOME/oslo/secrets/secrets.db
```

or, when `XDG_DATA_HOME` is unset:

```text
$HOME/.local/share/oslo/secrets/secrets.db
```

The directory is mode `0700`; the database is mode `0600`. The vault is global to the user and is
not switched with history profiles. A missing `XDG_DATA_HOME` and `HOME` is an explicit error for
`oslo secret`; unlike disposable tracking data, secret storage must never fail silently.

The age reference module derives these adjacent paths without configuring the Rust tool:

```text
secrets.db       ordinary database, present while open
secrets.db.age   persistent encrypted file
secrets.db.new   staged encrypted replacement
secrets.lock/    exclusive lock directory while open
```

### CLI

Implement exactly these first-version forms:

```text
oslo secret path
oslo secret status
oslo secret put NAME [--yes]
oslo secret get NAME
oslo secret list [PREFIX]
oslo secret rm NAME [--yes]
oslo secret help
oslo secret SUBCOMMAND --help
```

Rules:

- `path` prints the default plaintext database path and creates nothing.
- `status` prints the path, whether the plaintext database exists, its schema, entry count, and
  file size. It creates nothing and never prints a name or value.
- `put NAME` accepts no value argument. With terminal input it uses the existing `oslo-ui` input
  widget with `password = true` and `required = true`. With non-terminal stdin it reads the exact
  bytes up to EOF, rejects an empty value, and stores no implicit newline of its own.
- `put` replaces an existing value only after an interactive confirmation. In a non-terminal
  invocation, replacement requires `--yes`; add `put NAME --yes` to the parser even though the
  overview keeps the common form short.
- `get NAME` writes the exact stored bytes to stdout without adding a newline. Diagnostics go to
  stderr. This is an explicit reveal operation: on a terminal the value enters terminal output and
  scrollback.
- `list [PREFIX]` writes validated names only, one per line, in bytewise sorted order. It never
  reads values into the result collection.
- `rm NAME` confirms interactively. `--yes` is required without a terminal.
- Missing names are errors. Unknown options/subcommands and malformed usage exit `2`; storage,
  prompt, or lookup failures exit `1`; success exits `0`.
- Help uses the same `Paint`, headings, row alignment, terminal detection, and subcommand-help style
  as `oslo history`.
- Secret values must never appear in argv, error strings, debug output, JSON, tests, terminal mark
  payloads, or help examples.
- Do not add `run`, environment injection, stdin injection into child programs, clipboard support,
  editing, sync, export, import, or backup in this first version. They are valuable only after the
  basic storage and privacy boundary is proven, and each adds binary code and leak paths.

### Names and values

Names are metadata. Validate them before opening a write transaction:

```text
[A-Za-z0-9][A-Za-z0-9._/-]{0,127}
```

Additionally reject:

- absolute paths;
- empty path components;
- `.` and `..` components;
- names ending in `/`;
- NUL and non-ASCII bytes.

Values are opaque bytes. Limit one value to 1 MiB so a mistaken pipe cannot make the shell allocate
without bound. Do not trim, normalize, Unicode-normalize, append, or remove bytes.

### Database format

Use a separate Tagdata file through the existing `oslo-base` key-value seam. No other module may
`use tagdata` directly.

The database contains:

```text
meta/kind     = "oslo-secrets"
meta/schema   = 1
secret/NAME   = VALUE
```

Opening an existing file for secrets must refuse:

- a non-Tagdata file;
- a Tagdata page size Oslo does not support;
- a missing or different `meta/kind` marker;
- a newer schema;
- a symlink at the database path.

Creating the database and writing its marker plus first value happen under one checked write path.
Read-only commands never create an empty database. Storage errors are returned with the path and
operation but never the secret value.

### Automatic privacy gate

The automatic layer classifies a completed interactive command before any Oslo-owned persistence or
frecency update. A private command executes unchanged.

Sensitivity has three states:

```rust
pub enum Sensitivity {
    Public,
    ExplicitPrivate,
    Detected { rule: String },
}
```

`ExplicitPrivate` is the existing leading-space convention. It always wins. A pre-command hook may
replace the command; classify the replacement too and only promote sensitivity, never downgrade it.

For a private command Oslo must not:

- add it to the editor history or `$HISTFILE`;
- add it to recall, Tagdata history, outcomes, tracking, prediction, or repair inputs;
- count its command words toward frecency;
- expose it in terminal semantic marks, titles, slow-command notices, or chain reports;
- print expanded arguments through `set -x`.

The command may still be passed to `pre-cmd` and `post-cmd`. These hooks are executable user
configuration and form a trusted control boundary; the age module cannot operate if the general
pre-command hook is denied the parsed command it must recognize. Document this exception clearly.

The initial high-confidence detector rules are:

| Rule ID | Match |
|---|---|
| `private-key` | a PEM private-key header |
| `authorization-header` | case-insensitive `Authorization:` with a non-empty value |
| `credential-url` | URI authority containing non-empty `user:password@` |
| `secret-assignment` | assignment name containing `KEY`, `PASS`, `SECRET`, or `TOKEN` with a non-empty value |
| `secret-option` | `--password`, `--passwd`, `--token`, `--secret`, `--api-key`, `--auth`, `--bearer`, or `--credential` with a value |
| `curl-user-password` | `-u` followed by non-empty `user:password` |
| `github-token` | a known GitHub token prefix and plausible body |
| `aws-access-key` | the documented `AKIA` shape and length |
| `jwt` | three non-empty base64url-shaped sections with a JSON-header-shaped first section |

Do not suppress history merely because a line is long, multiline, invokes `age`/`gpg`, contains the
word `secret`, or has generic high entropy. `oslo secret get NAME` contains metadata, not the secret
value, and must remain visible to the imported hook module.

Expose detector tuning under a separate privacy namespace, never under `oslo.secret`:

```lua
oslo.privacy = {
    detect = true,
    notice = true,
    allow = {},
    patterns = {},
}
```

- `detect`: default `true`.
- `notice`: default `true`; prints only `private command hidden (<rule-id>)`.
- `allow`: built-in rule IDs to disable. Unknown IDs are configuration errors.
- `patterns`: public rule ID to regex source. Compile once at interactive startup, sort by rule ID,
  and report invalid patterns during configuration.
- Pattern text and matched text never appear in a notice.

## General `secrets.lua` contract

The reference file is ordinary configuration and must use only public Oslo Lua APIs plus the
external `age` executable. It must not require Rust changes when a user swaps commands.

### Command recognition

Use `pre-cmd`'s parsed `commands` table; do not split `command.text`. Match only a single,
foreground, simple command whose literal argv begins:

```text
argv[1] == "oslo"
argv[2] == "secret"
```

Wrap only subcommands that may open the database: `status`, `put`, `get`, `list`, and `rm`. Do not
unlock for `path`, `help`, or `--help`.

The current parsed-command payload loses whether the final list item used `&`. Extend
`crates/oslo-runtime/src/lua/parsed.rs` generically with a boolean `background` field and tests.
The reference module refuses background commands, pipelines, `&&`, `||`, `;`, multiple commands,
aliases, variable command names, and nonliteral spellings. The safe supported form is a standalone:

```text
oslo secret SUBCOMMAND ...
```

Do not keep the plaintext database open while unrelated commands run.

### Pre-command lifecycle

The module keeps `active = false` in its Lua closure. When it recognizes a supported command:

1. Refuse if `active` is already true.
2. Atomically acquire `secrets.lock/` with an external `mkdir` argv call. `oslo.fs.mkdir` is `-p`
   and therefore cannot be used as the exclusive acquisition primitive.
3. Write the Oslo PID and timestamp as non-secret diagnostic metadata inside the lock directory.
4. Refuse if plaintext `secrets.db` already exists. Never overwrite crash residue.
5. If `secrets.db.age` exists, run age directly with argv to decrypt it to `secrets.db`.
6. If the encrypted file is absent, permit only `put`; it will create the ordinary database.
7. Restrict the plaintext database to mode `0600` when it exists.
8. Set `active = true` only after every precondition succeeds.
9. Return the original `command.text`, not `nil`. `pre-cmd` uses first-answer-wins semantics; claiming
   the command prevents a later pre-command handler from cancelling it after plaintext was opened,
   which would skip `post-cmd` in the current REPL.

Any failure before step 8 removes a lock acquired by this invocation, prints a value-free error,
and returns `false` to cancel execution.

The import must be the first pre-command handler the user's config attaches. State this beside the
`require("secrets")` line because hook attachment order is the only ordering mechanism Oslo has.

### Post-command lifecycle

The general `post-cmd` handler ignores command text and acts only when its own `active` flag is true:

1. Set `active = false` before starting cleanup so a failure cannot make a later unrelated command
   look owned by this lifecycle.
2. Confirm plaintext `secrets.db` exists and is a regular non-symlink file.
3. Refuse an existing or symlinked `secrets.db.new`; never let age overwrite unknown staging data.
4. Run age directly with argv to encrypt it to `secrets.db.new` using configured public recipients.
5. Set mode `0600` on the staged ciphertext.
6. Atomically rename `secrets.db.new` over `secrets.db.age` only after age succeeds.
7. Remove plaintext `secrets.db` only after the encrypted replacement exists.
8. Remove `secrets.lock/` last.

Never delete or overwrite the previous `secrets.db.age` before the staged encryption succeeds. On
post-hook failure, preserve the previous ciphertext, the plaintext database, and the lock directory
for explicit recovery, and print a loud diagnostic naming paths but no values.

Current `post-cmd` is observational, so a failing post hook cannot replace the already completed
command's exit status. Do not change the global hook contract in this plan. Document that a close
failure is reported loudly and leaves the vault locked for recovery even though the original tool
status is unchanged.

### Reference age configuration

`docs/features/secrets.lua` must keep all policy as editable Lua constants near its top:

- plaintext database path;
- encrypted file path;
- staging file path;
- lock directory path;
- age executable;
- age identity path;
- age recipients file or recipient arguments.

It passes paths and recipients as argv elements through `oslo.run{...}`. It never uses `sh -c`,
`os.execute`, `io.popen`, command concatenation, a secret environment variable, or a secret argv
argument. The module handles only age as a concrete example; its accompanying documentation shows
that GPG/custom behavior is achieved by replacing the two external argv calls, not by configuring
the Rust tool.

## Threat model

### Protected

- Secret values entered through masked `put` are absent from the command line and Oslo history.
- High-confidence accidental inline secrets are excluded from Oslo persistence and suggestions.
- The secret database is separate from all history profiles and history maintenance.
- With the reference module, the previous encrypted vault stays valid until replacement succeeds.
- Concurrent interactive Oslo shells using the same module cannot both open the vault.
- Oslo itself contains no cryptographic implementation, private key, provider SDK, or crypto crate.

### Not protected

- Plaintext typed into the ordinary editor before Enter; the terminal already drew it.
- The raw PTY transcript of a scratch session, screen recording, or terminal scrollback.
- `oslo secret get NAME` output intentionally written to stdout.
- General `pre-cmd` and `post-cmd` handlers, which are trusted executable configuration.
- A malicious or replaced `age` executable, Lua module, or target executable.
- Same-user processes while `secrets.db` exists in plaintext.
- Filesystem journals, snapshots, SSD remanence, swap, or backups that capture the temporary file.
- `SIGKILL`, power failure, or a shell crash between pre and post hooks. These may leave plaintext
  plus `secrets.lock/`; the next invocation must refuse to overwrite them.
- Direct `oslo secret` invocations from Bash, scripts, `sh -c`, or another noninteractive caller.
  Oslo configs and their general hooks load only in the interactive REPL. Such an invocation sees
  the ordinary plaintext database if one exists and cannot unlock `secrets.db.age` by itself.
- Root, kernel compromise, ptrace/memory inspection where permitted, or perfect memory erasure.

This feature must be described as history safety plus optional encrypted-at-rest configuration, not
as process isolation or a complete secrets manager security boundary.

## Current-state evidence

- `src/cli/tools.rs:28-62` is the single list of `oslo <tool>` names and help descriptions.
- `src/cli/tools.rs:114-144` dispatches `history` and `scratch`; `secret` needs the same explicit
  route rather than falling into stub help.
- `src/cli/history/help.rs` is the required subcommand-help renderer and style exemplar.
- `crates/oslo-base/src/track/mod.rs:40-45` states that nothing outside `track::kv` may import
  Tagdata. Preserve that engine boundary.
- `crates/oslo-base/src/track/kv/mod.rs:30-105` owns bucket names through `Tree`.
- `crates/oslo-base/src/track/kv/file.rs:10-64` already owns mode `0600`, directory mode `0700`,
  page-size validation, and database-file preparation.
- `crates/oslo-ui/src/ask/input.rs:20-58` already provides masked, required terminal input.
- `crates/oslo-runtime/src/startup/read.rs:313-369` derives the leading-space flag and currently
  records frecency before returning the command.
- `crates/oslo-runtime/src/startup/repl.rs:238-263` writes editor history, recall, and Tagdata before
  the existing `pre-cmd` hook.
- `crates/oslo-runtime/src/startup/repl.rs:283-303` shows that `pre-cmd` is first-answer-wins and
  cancellation currently skips directly to the next prompt.
- `crates/oslo-runtime/src/startup/repl.rs:419-426` fires `post-cmd` after execution and on failures.
- `crates/oslo-runtime/src/lua/api/hooks.rs:1-28` documents the existing general pre/post command
  hooks; no new secret-specific moment is needed.
- `crates/oslo-runtime/src/lua/parsed.rs:44-110` supplies parsed argv to `pre-cmd` but currently
  loses the final background bit.
- `crates/oslo-lua/src/stdlib/module.rs:21-40` already searches
  `$XDG_CONFIG_HOME/oslo/lua/?.lua`, which is why `require("secrets")` needs no loader changes.

## Scope

### In scope

- `crates/oslo-base/src/lib.rs`
- `crates/oslo-base/src/privacy.rs` (new)
- `crates/oslo-base/src/secret/mod.rs` (new)
- `crates/oslo-base/src/secret/store.rs` (new)
- `crates/oslo-base/src/secret/tests.rs` (new, if splitting tests keeps files below the LOC gate)
- `crates/oslo-base/src/track/kv/mod.rs`
- `crates/oslo-base/src/track/kv/file.rs`
- `crates/oslo-runtime/src/startup/read.rs`
- `crates/oslo-runtime/src/startup/repl.rs`
- `crates/oslo-runtime/src/startup/editor.rs`
- `crates/oslo-runtime/src/startup/notify.rs`
- `crates/oslo-runtime/src/startup/tracking.rs`
- `crates/oslo-runtime/src/lua/parsed.rs`
- `crates/oslo-shell/src/exec/simple/trace.rs`
- `crates/oslo-ui/src/settings/mod.rs`
- `crates/oslo-ui/src/settings/from_lua.rs`
- `crates/oslo-ui/src/settings/privacy.rs` (new if consistent with the current settings split)
- `src/cli.rs`
- `src/cli/tools.rs`
- `src/cli/tools/tests.rs`
- `src/cli/secret.rs` (new)
- `src/cli/secret/help.rs` (new)
- `src/cli/secret/tests.rs` (new)
- `tests/secret_cli_tests.rs` (new)
- `tests/secret_privacy_tests.rs` (new)
- `tests/terminal_semantics_tests.rs` and a focused file under `tests/terminal_semantics/` if needed
- `tests/config_source_tests.rs`
- `docs/features/secrets.md` (new)
- `docs/features/secrets.lua` (new, the tested reference module)
- `plans/PLAN_SECRETS.md` status/checklist updates only

### Out of scope

- `README.md` and every existing README file. Do not add secret documentation there.
- `PLAN.md`, `PLAN_DIVERGES.md`, and other plans.
- `examples/`; it was intentionally removed and must not be recreated.
- Cargo dependency additions or feature flags.
- Native encryption, crypto libraries, provider libraries, D-Bus, TLS, async runtimes, daemons, or
  network access.
- A bare `secret` builtin or changes to builtin registration.
- History database schema, profile layout, sync protocol, import/export format, or existing history
  CLI behavior.
- Generic hook ordering, priority, or post-hook status semantics beyond the one parsed-command
  `background` field required by the configuration module.
- Clipboard integration, environment injection, child execution, shell substitution shortcuts,
  output masking, global redaction, secure deletion claims, and automatic extraction of secrets
  from detected commands.
- Scratch/tab source and UI refactors unrelated to masked input.

## Git workflow

- Work on a dedicated feature branch created from the current `develop` unless the operator says
  otherwise.
- Commits are title-only Conventional Commits, no body, no signature, no co-author line.
- Format: `<type>(<optional-scope>): <message>` with the full title at most 50 characters.
- Suggested logical commits:
  - `feat(secret): add plain secret database`
  - `feat(privacy): hide sensitive commands`
  - `docs(secret): add age hook module`
- Do not push, merge, or open a pull request unless explicitly requested.

## Commands

Use the Makefile for every relevant repository task.

| Purpose | Command | Expected success |
|---|---|---|
| Fast compile | `make check` | exit 0 |
| Full tests | `make test` | exit 0, all Oslo workspace tests pass |
| Terminal tests | `make test-terminal` | exit 0 |
| Formatting check | `make fmt-check` | exit 0 |
| Lint | `make clippy` | exit 0, no warnings |
| Documentation compile | `make rustdoc` | exit 0, no warnings |
| LOC policy | `make check-loc` | exit 0 |
| Documentation paths | `make check-readme` | exit 0 |
| Full gate | `make verify` | exit 0 |
| Minimal static release | `make build TYPE=minimal` | exit 0; no INTERP or NEEDED entries |

Do not use Python to generate, edit, migrate, or inspect files. Use Rust tests, shell commands, and
patch/edit tools.

## Implementation phases

### Phase 0: Record baseline and reconcile drift

1. Run the drift command from the header.
2. Run `git status --short` and record unrelated user changes; never overwrite them.
3. Run `make test`, `make test-terminal`, and `make build TYPE=minimal`.
4. Record the exact minimal binary byte count from
   `target/x86_64-unknown-linux-musl/release/oslo` in this plan's implementation notes or commit
   handoff.
5. Confirm `age` is not needed for Rust unit tests; hook integration tests use a deterministic fake
   executable and synthetic values.

**Verify**: all three Make targets exit `0`; baseline bytes are recorded.

### Phase 1: Characterize privacy sinks and hook behavior

Before changing production behavior, add regression tests proving the current seams:

- an accepted command reaches editor history, recall, Tagdata log/outcome, tracking, and frecency;
- a leading-space command already skips history/recall/tracking but currently still reaches the
  early frecency call, characterizing the gap Phase 4 must close;
- `pre-cmd` sees parsed argv for `oslo secret get synthetic-name`;
- returning a string from a pre-command handler prevents later handlers from running;
- cancellation after a handler would skip `post-cmd`, documenting why `secrets.lua` must claim the
  command after opening;
- `post-cmd` fires after success and failure;
- the parsed command table reports the new final `background` boolean correctly.

Use synthetic markers such as `synthetic-secret-marker`; never use realistic credentials.

**Verify**: `make test` and `make test-terminal` exit `0`.

### Phase 2: Extend the checked Tagdata seam and add the plain secret store

1. Add a checked create/open function to `track::kv` that returns `Result<Store, String>` with the
   real path-bearing error. Keep the existing optional tracking open behavior unchanged.
2. Refuse symlinks before opening or creating a secret database path.
3. Add `Tree::Secret` and update the exhaustive bucket table/count.
4. Add `oslo_base::secret` without importing Tagdata directly.
5. Implement default-path derivation, name validation, schema marker validation, size limit, exact
   byte storage, sorted prefix listing, checked put/get/remove, status, and private permissions.
6. Keep read-only opens non-creating. A first `put` may create and initialize the database.
7. Test wrong kind, future schema, foreign page size, symlink, missing home, exact bytes, overwrite,
   prefix ordering, removal, size limit, and file modes.

**Verify**: `make check` and `make test` exit `0`.

### Phase 3: Implement `oslo secret`

1. Register `secret` in `src/cli/tools.rs` and route it explicitly to `src/cli/secret.rs`.
2. Do not add a builtin registration.
3. Implement the exact CLI contract and exit statuses above.
4. Use the existing masked input widget for terminal `put`; use exact stdin bytes for non-terminal
   `put`.
5. Render overview and subcommand help through the same style as history.
6. Add integration tests under a temporary `HOME`/`XDG_DATA_HOME` for every subcommand, first-put
   creation, exact output, errors, modes, help, no-value argv rejection, no creation by read-only
   commands, and independence from history profiles.
7. Add a PTY test proving the entered synthetic value does not appear in the terminal transcript
   during masked `put`.

**Verify**: `make test` and `make test-terminal` exit `0`.

### Phase 4: Add one privacy classification path

1. Add `Sensitivity`, `Detector`, rule IDs, custom rule compilation, and a thread-local execution
   privacy guard in `oslo-base/src/privacy.rs`.
2. Add `oslo.privacy` settings using the existing settings conversion/error pattern.
3. Classify a completed expanded command in `startup/read.rs` before calling
   `record_command_use`; skip frecency when private.
4. Replace `Input::Command.secret: bool` with `sensitivity: Sensitivity`.
5. Use `sensitivity.is_private()` for editor history, recall, Tagdata append/outcome, tracking,
   `pre-record`, and terminal semantic marks.
6. Reclassify a `pre-cmd` string replacement and promote the original sensitivity.
7. For private execution, enter the privacy guard around evaluation. Make xtrace consult this guard
   and emit nothing while it is active without changing the user's `set -x` option state.
8. Do not arm pipeline segment/chain recording for a private command.
9. Replace title and slow-command text with a constant private label; never include the rule pattern
   or match.
10. Keep full command payloads for the trusted general `pre-cmd` and `post-cmd` boundary and test
    that the imported module can still recognize `oslo secret`.

**Verify**: `make check`, `make test`, and `make test-terminal` exit `0`.

### Phase 5: Add and test the imported age hook module

1. Extend parsed command metadata with the generic final `background` fact and document it in
   `docs/features/hooks.md` only if that existing document must describe the public field. Do not
   touch a README.
2. Add `docs/features/secrets.lua` implementing the lifecycle exactly as specified.
3. Add `docs/features/secrets.md` explaining the plain core, final user paths, the one-line import,
   age constants, no-encryption choice, GPG/custom replacement, supported standalone command shape,
   crash residue, manual recovery, and honest threat boundary.
4. In integration tests, copy the exact repository `docs/features/secrets.lua` into a temporary
   `$XDG_CONFIG_HOME/oslo/lua/secrets.lua`; do not duplicate its body as a Rust string.
5. Put `require("secrets")` first in the temporary `config.lua`.
6. Put a deterministic fake `age` executable first on the temporary `PATH`. It copies synthetic
   bytes and records only operation/path order, never a value.
7. Prove pre/decrypt/tool/post/encrypt/delete order, first `put`, failed decrypt cancellation,
   failed encryption preservation, lock contention across two interactive shells, refusal of
   background/pipeline/chained forms, residue refusal, and the no-import plaintext path.

**Verify**: `make test` and `make test-terminal` exit `0`.

### Phase 6: Full security and size gate

1. Run `make verify`.
2. Run `make build TYPE=minimal` and record the exact after byte count and delta from Phase 0.
3. Confirm no dependency or feature was added to any Cargo manifest or lockfile.
4. Inspect the final binary with the existing Makefile static check.
5. Search the diff for realistic token prefixes, private-key text, value-bearing examples, shell
   command concatenation, `sh -c`, `oslo.secret`, and direct `use tagdata` outside the existing seam.
6. Run `git diff --stat` and confirm every changed source file is in scope.

The minimal release increase must be reported. If it exceeds 131,072 bytes, STOP and request a size
review rather than committing the implementation. Do not hide size by removing unrelated features
or weakening tests.

**Verify**: `make verify` and `make build TYPE=minimal` exit `0`; static check passes; delta is at
most 131,072 bytes or the work is stopped for review.

## Test matrix

### Plain store

- first `put` creates the right path and schema;
- `path`, `status`, `get`, and `list` do not create a missing database;
- exact bytes round-trip, including spaces and a final newline from piped stdin;
- 1 MiB accepted, larger refused without writing;
- overwrite confirmation/`--yes` behavior;
- sorted prefix list contains names only;
- invalid names rejected before any write;
- missing/wrong kind, future schema, wrong page size, and symlink refused;
- directory `0700`, file `0600`;
- history profile changes do not change the secret path.

### CLI and terminal

- tool is listed and safely dispatched through the operand rule;
- every subcommand and subcommand help page follows main help styling;
- unknown subcommand/option is status `2`;
- missing value is status `1`, with no stdout;
- `get` writes exact bytes and no implicit newline;
- masked `put` transcript contains bullets or UI chrome but not the synthetic value;
- no value is accepted as a command-line operand.

### Privacy

- each built-in detector rule has positive and near-miss tests;
- disabled rules and sorted custom patterns are deterministic;
- notice contains only rule ID;
- leading-space explicit privacy wins;
- pre-hook replacement may promote but not downgrade privacy;
- private commands execute unchanged;
- private commands do not reach history, recall, Tagdata log/outcome, tracking, frecency, chain,
  semantic marks, titles, notices, or xtrace;
- public `oslo secret get synthetic-name` remains recognizable to general pre/post hooks;
- trusted command hooks still receive command metadata, as documented.

### Imported `secrets.lua`

- `require("secrets")` resolves from `$XDG_CONFIG_HOME/oslo/lua`;
- standalone foreground recognition uses parsed argv;
- help/path do not unlock;
- encrypted existing vault follows lock/decrypt/tool/encrypt/rename/delete/unlock order;
- first `put` works without an existing encrypted file;
- another shell is refused while lock directory exists;
- failed pre-hook cancels the command and cleans only its own partial state;
- failed post encryption keeps old ciphertext, plaintext, and lock for recovery;
- stale plaintext is never overwritten;
- chained, pipeline, multiple-command, and background forms are refused;
- without import, the ordinary plaintext database works directly.

## Done criteria

- [ ] `oslo secret` is a CLI tool, not a builtin.
- [ ] Its Rust implementation opens only the ordinary default plaintext database.
- [ ] No encryption/provider/codec/key/hook field exists under `oslo.secret`.
- [ ] No new Cargo dependency or feature exists.
- [ ] Tagdata remains imported only through the existing key-value seam.
- [ ] Values never enter argv and round-trip exactly.
- [ ] Read-only commands do not create a database.
- [ ] File and directory permissions are tested.
- [ ] Automatic private classification happens before frecency and persistence.
- [ ] Every listed Oslo-owned sink has a negative-leak regression test.
- [ ] General pre/post command hooks remain the trusted configuration boundary.
- [ ] `docs/features/secrets.lua` is the exact module integration tests execute.
- [ ] User config imports it with `require("secrets")`.
- [ ] Age is an external example only; no age code or key handling is in Rust.
- [ ] Crash, same-user, scrollback, direct-CLI, filesystem-remanence, and hook-order limits are
  documented honestly.
- [ ] No README file, `PLAN.md`, or `examples/` path changed.
- [ ] `make verify` passes.
- [ ] `make build TYPE=minimal` is static and the exact size delta is reported.
- [ ] The size delta is at most 131,072 bytes or implementation stopped for review.

## STOP conditions

Stop and report instead of improvising if:

- current code no longer matches the architectural evidence after drift review;
- implementing the plain store appears to require a new database or crypto dependency;
- any implementation needs a secret value in argv, a shell command string, a log, or a temporary
  test fixture committed with realistic credentials;
- the only proposed solution moves encryption, age, providers, pre-hooks, or post-hooks into
  `oslo secret` or an `oslo.secret` configuration table;
- the implementation requires a bare `secret` builtin;
- the age module cannot guarantee post cleanup after successful pre setup for the supported
  standalone foreground shape;
- a later pre-command handler can cancel after the module opened plaintext; fix the module's
  first-answer ownership or stop;
- a failed age encryption would overwrite the old ciphertext or delete the only recoverable
  plaintext copy;
- tests require the real age executable, a real identity, a network service, or a real credential;
- satisfying documentation checks appears to require editing any README file;
- a relevant Make target fails twice after a reasonable correction;
- the minimal static binary grows by more than 131,072 bytes;
- an in-scope source file exceeds the repository LOC policy and cannot be split cleanly.

## Review checklist

- Read the Rust secret module without the Lua documentation: it should look like an ordinary,
  strict key-value database API with no hint of age or hooks.
- Read `docs/features/secrets.lua` without the Rust source: it should treat `oslo secret` as an
  external command that happens to open the configured plaintext path.
- Confirm the module uses parsed argv and direct argv execution, not textual splitting or shell
  concatenation.
- Confirm lock acquisition precedes decryption and lock release follows plaintext removal.
- Confirm old ciphertext survives every failed close path.
- Confirm a pre-open cancellation cannot bypass the post hook.
- Confirm private detection changes observation only, never the command text or execution result.
- Confirm hook visibility is documented as trusted rather than falsely claimed absent.
- Confirm every diagnostic can be produced without formatting a secret value.
- Confirm binary-size evidence compares the same minimal build before and after.

## Maintenance notes

- `docs/features/secrets.lua` is policy code. Any change to hook payloads, first-answer semantics,
  module paths, `oslo.run`, or `oslo.fs` must run its integration tests.
- The general hooks are interactive-only. If future work wants encrypted access from Bash or
  scripts, design a general external wrapping mechanism separately; do not smuggle it into the
  plain database tool.
- A future `run` subcommand should inject a selected value without putting it in argv and needs its
  own threat model and size measurement.
- A future daemon could hold an unlocked vault or kernel lock, but it is not part of this model and
  would not create trustworthy same-user per-application authorization by itself.
- If the Tagdata seam moves out of `track::kv`, move both tracking and secrets together so direct
  engine imports do not multiply again.
- Secure deletion is not promised. Moving transient plaintext to a runtime filesystem can be
  considered later, but the agreed first model deliberately gives `oslo secret` one ordinary,
  stable database path.
