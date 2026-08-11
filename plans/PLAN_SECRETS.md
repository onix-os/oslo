# Plan: Add history-safe secret handling to Oslo

> **Executor instructions**: Follow the phases in order. Run every verification gate before moving
> on. Do not implement adjacent refactors. If a STOP condition occurs, stop and report it instead of
> inventing a different security model. Never use a real credential in a test, log, commit, issue or
> example.
>
> **Drift check (run first)**:
>
> ```sh
> git diff --stat f9d15a2..HEAD -- \
>   crates/oslo-base/src \
>   crates/oslo-runtime/src/startup \
>   crates/oslo-runtime/src/lua/engine.rs \
>   crates/oslo-shell/src/env/builtins \
>   crates/oslo-shell/src/exec \
>   crates/oslo-ui/src/settings \
>   tests
> ```
>
> Compare the current code with the evidence in this plan if any in-scope file changed. The checkout
> was actively receiving unrelated tab/UI refactors while this plan was authored. All tab source and
> `crates/oslo-ui/src/ask/choose.rs` are out of scope and must remain untouched.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: the active tab refactor being committed or moved aside
- **Category**: security
- **Planned at**: commit `f9d15a2`, 2026-08-11
- **Status**: TODO

## Outcome

After this plan is implemented, Oslo has two separate protections:

1. **An automatic privacy gate** classifies a completed interactive command before Oslo remembers,
   observes or describes it. A detected command executes unchanged but its text is omitted from
   Oslo-owned history, recall, tracking, prediction inputs, command metadata, hooks, titles,
   notifications, chain reports and xtrace.
2. **A `secret` shell builtin** accepts values through the existing masked input widget, stores them
   through an external secure provider, and supplies named values to an external child through an
   explicit environment variable or stdin. The value is never an Oslo command argument and never
   interpolated into shell source.

The implementation must describe itself as a safety net, not a complete leak-prevention boundary.
Plaintext typed into the ordinary editor has already been drawn by the terminal before Enter. With
the `tab` feature, that draw may already exist in the raw PTY transcript. The strong path is masked
entry followed by a named reference.

## Product contract

### Automatic detection

Given this shape of interactive command:

```text
true --token=<synthetic-test-marker>
```

Oslo must:

- run the command without textual rewriting;
- optionally report only `secret: command hidden (<rule-id>)`;
- never print the matched value in its diagnostic;
- not add the line to `$HISTFILE`, in-memory history, recall or Tagdata;
- not feed the line to frecency, tracking, prediction or repair;
- not expose it through semantic command metadata, prompt titles, notifications, Lua observer hooks,
  chain reports or `set -x`;
- preserve the leading-space private-history convention as an explicit override;
- remain conservative: a false positive costs a missing history entry, while execution is unchanged.

Detection does not vault the value. There is no automatic extraction, naming or later reveal of a
value that happened to appear in a command.

### `secret` builtin

The first implementation exposes exactly these forms:

```text
secret put NAME
secret edit NAME
secret list [PREFIX]
secret show NAME
secret rm NAME [--yes]
secret run [--env VAR=NAME]... [--stdin NAME] -- PROGRAM [ARGUMENT]...
secret status
secret help
secret SUBCOMMAND --help
```

Rules:

- `put` and `edit` accept no value argument and use masked required input.
- `show` requires an interactive terminal and confirmation. Its prompt explicitly says the value
  will enter terminal scrollback and, inside a tab, the tab transcript.
- `rm` confirms interactively; `--yes` is required when no terminal is available.
- `list` prints names managed by Oslo, never provider values.
- `status` prints the selected provider, whether its executable is available, and the metadata-index
  path. It never performs a lookup and never prints a value.
- `run` executes an external program only. It does not dispatch shell functions, aliases or builtins.
  A user needing shell syntax writes `secret run ... -- sh -c '...'` explicitly.
- `--env VAR=NAME` may be repeated. `VAR` must be a valid exported environment name.
- `--stdin NAME` may appear at most once. It sends the exact stored bytes and then closes stdin; it
  does not append a newline.
- Provider and child programs are invoked directly with argv arrays, never through `/bin/sh -c`.
- Secret names are metadata. Reject names outside `[A-Za-z0-9][A-Za-z0-9._/-]{0,127}`, including
  `..` path components, empty path components and absolute paths.
- There is deliberately no `secret get`, command-substitution shortcut, clipboard command, global
  output masker or automatic provider fallback in the first version.

## Threat model and guarantees

### Protected

- Accidental persistence in Oslo's flat history, recall and Tagdata history.
- Later exposure through history search, export, backup or sync because sensitive lines never enter
  the underlying store.
- Oslo-generated titles, notifications, semantic marks, hook payloads, chain summaries and xtrace.
- Secret values appearing in provider or target-process argv.
- Secret values remaining exported in the parent Oslo process after `secret run`.

### Not protected

- A value typed directly into the ordinary editor before Enter.
- Terminal scrollback, screen recording, terminal multiplexers and the current raw tab transcript.
- Output intentionally produced by the target command.
- A child reading another same-user process's environment where the operating system allows it.
- A malicious provider executable or malicious target program.
- Core dumps, swap, kernel compromise or perfect memory erasure.
- User configuration explicitly granting an observer access to data outside the redacted hook API.

Use stdin injection where a target supports it. Environment injection is a compatibility mechanism,
not a stronger secrecy boundary.

## Decisions already made

### Do not textually wrap accepted commands

The automatic layer sets execution context equivalent to:

```text
execute(command, sensitivity = Detected(rule_id))
```

It must not create a new string such as `secret run -- ...`. Textual wrapping changes how shell
assignments, builtins, functions, aliases, redirects, pipelines and control operators behave.

### Do not create a native encrypted database

Provider values live outside Tagdata and outside a new Oslo-specific vault. The first providers are
external programs:

1. `secret-service`, implemented through `secret-tool`;
2. `pass`, implemented through the `pass` executable.

No provider crate, D-Bus crate, TLS stack, crypto crate or async runtime may be added for this plan.

The default provider is `secret-service`. If `secret-tool` is unavailable or no Secret Service is
running, the builtin reports the problem and suggests configuring `pass`. It must not silently store
the same name in another provider.

### Store only a managed-name index locally

`secret list` reads an Oslo-owned metadata index. It does not use `secret-tool search`, because that
operation can return secret details while listing. The index contains only validated names and the
provider that owns each name.

Path:

```text
$XDG_DATA_HOME/oslo/secrets.index
```

or, when `XDG_DATA_HOME` is unset:

```text
$HOME/.local/share/oslo/secrets.index
```

The directory is mode `0700`, the file is mode `0600`, writes use a sibling temporary file followed
by rename, and symlinked files are refused. `put` adds or updates the index only after provider
success. `rm` removes the row only after provider success. Provider changes made outside Oslo may
make the index stale; `show` and `run` still treat the provider lookup as authoritative.

Secret names and provider lookup attributes are not confidential. The Secret Service specification
allows attributes to be stored unencrypted, so names must never contain secret material.

## Configuration

Add a root `oslo.secret` table because the settings span both the interactive REPL and the builtin:

```lua
oslo.secret = {
    provider = "secret-service", -- or "pass"
    detect = true,
    notice = true,
    allow = {},
    patterns = {
        -- company_token = "a high-confidence regular expression",
    },
}
```

Exact semantics:

- `provider`: `secret-service` or `pass`; another value is a configuration error and retains the
  default.
- `detect`: enables accept-time automatic detection; default `true`.
- `notice`: prints the safe rule-ID notice for automatically detected lines; default `true`.
- `allow`: a list of built-in rule IDs to disable. Unknown IDs are configuration errors.
- `patterns`: a mapping from a public rule ID to a regex. IDs use lowercase letters, digits, `_` and
  `-`. Invalid regexes are reported during configuration, not after Enter.

Sort custom patterns by rule ID before compiling them so Lua table iteration cannot change which
rule is reported. The pattern text must never appear in the detection notice.

## Detection rules

Create a small deterministic rule set. Do not import a large provider database. Built-in rule IDs:

| Rule ID | Match |
|---------|-------|
| `private-key` | PEM private-key headers or a complete private-key block start |
| `authorization-header` | case-insensitive `Authorization:` carrying a non-empty value |
| `credential-url` | a URI authority containing a non-empty `user:password@` pair |
| `secret-assignment` | an assignment whose name contains `KEY`, `PASS`, `SECRET`, or `TOKEN` and whose value is non-empty |
| `secret-option` | `--password`, `--passwd`, `--token`, `--secret`, `--api-key`, `--auth`, `--bearer`, or `--credential`, with a glued or following non-empty value |
| `curl-user-password` | `-u` followed by a non-empty `user:password` pair |
| `github-token` | known GitHub token prefixes followed by a plausible non-empty body |
| `aws-access-key` | `AKIA` followed by the expected uppercase/digit shape and length |
| `jwt` | three non-empty base64url-shaped sections separated by dots, with a JSON-header-shaped first section |
| `custom-<id>` | one configured local regex |

Do not promote these existing tracking heuristics into full history suppression:

- a line merely being longer than 4096 bytes;
- a line being multiline;
- any leading assignment regardless of its name;
- any invocation of `gpg`, `openssl`, `pass`, `vault` or `secret-tool`;
- generic mixed-case/digit entropy without secret-related context.

Those rules currently reduce tracking detail, where a false positive is cheap. Suppressing the whole
interactive history entry has a higher usability cost and needs credential evidence.

## Internal design

### Sensitivity type

Add `crates/oslo-base/src/secret.rs` and export it from `crates/oslo-base/src/lib.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sensitivity {
    Public,
    ExplicitPrivate,
    Detected { rule: String },
}

impl Sensitivity {
    pub fn is_private(&self) -> bool;
    pub fn rule(&self) -> Option<&str>;
}
```

The module also owns `Detector`, built-in rule IDs and detection unit tests. `Detector` compiles custom
regexes once at interactive startup. It takes settings as plain strings so `oslo-base` does not depend
on `oslo-ui`.

`ExplicitPrivate` always wins. A pre-command hook replacement can promote `Public` to `Detected`, but
must never downgrade a private command.

### Accept-time placement

Change `crates/oslo-runtime/src/startup/read.rs` so a completed expanded buffer is classified before
`OsloHelper::record_command_use`. Replace `Input::Command.secret: bool` with
`Input::Command.sensitivity: Sensitivity`.

Preserve the original leading-space decision before trimming. Run content detection on the completed
expanded buffer, because that is what executes and what history currently stores.

### One privacy gate for all sinks

In `crates/oslo-runtime/src/startup/repl.rs`, derive one `private` boolean from `Sensitivity` and use it
for every sink. Do not let each subsystem re-detect the line.

For private commands:

- skip `editor::remember`;
- skip `remember_history`;
- skip Tagdata `append`, trim, settlement and outcome rows;
- skip `OsloHelper::record_command_use`;
- do not arm the chain recorder and do not print a resumable chain;
- call terminal `output_start(None)`;
- render the running title as `private`, without command text;
- suppress slow-command notifications and `on-report` notification handlers;
- forget tracking boundaries;
- suppress `set -x` for the full execution of the top-level private command.

Add an RAII trace-suppression guard in `crates/oslo-shell/src/exec/simple/trace.rs`, exposed through a
narrow function from `crates/oslo-shell/src/exec/mod.rs`. A depth counter is required so nested
evaluation cannot re-enable trace early. The default path must remain one cheap branch.

### Hooks

Extend the command-started and command-finished contexts in
`crates/oslo-runtime/src/lua/engine.rs` with:

```text
sensitive = true|false
secret_rule = rule-id or nil
```

For private commands, set `text` to the literal placeholder `<private>`. Never omit the field, because
existing Lua handlers may assume it is a string. The pre-command hook may still cancel or replace the
line. Reclassify a replacement before execution and taint it with the original sensitivity so a
private input cannot become public. Public commands keep the existing raw hook behavior.

### Tracking redaction

Keep structural tracking-only rules in `crates/oslo-base/src/track/redact.rs`. Replace duplicated
credential-specific checks with the shared detector where doing so preserves current head-only
tracking behavior. Tracking may remain stricter than full history suppression.

### Provider boundary

Add a `secret/` module under `crates/oslo-shell/src/env/builtins/`:

```text
secret.rs              command parser and help
secret/provider.rs     provider trait and selection
secret/secret_tool.rs  Secret Service argv protocol
secret/pass.rs         pass argv protocol
secret/index.rs        validated metadata index
secret/run.rs          external-child environment/stdin injection
secret/tests.rs        parser, provider and leak tests
```

The provider interface operates on bytes and never implements `Debug` for a value container:

```rust
trait Provider {
    fn put(&self, name: &SecretName, value: &[u8]) -> io::Result<()>;
    fn lookup(&self, name: &SecretName) -> io::Result<SecretValue>;
    fn remove(&self, name: &SecretName) -> io::Result<bool>;
    fn available(&self) -> bool;
}
```

Do not claim guaranteed memory erasure. Avoid cloning `SecretValue`, never implement `Display` or
`Debug` for it, keep its scope short and drop it immediately after the provider or child operation.

### Provider protocols

`secret-service` uses direct argv equivalent to:

```text
secret-tool store --label oslo:<name> oslo-app oslo oslo-name <name>
secret-tool lookup oslo-app oslo oslo-name <name>
secret-tool clear oslo-app oslo oslo-name <name>
```

The value for `store` is written to the child's stdin. Capture lookup stdout internally and remove
only one provider-added trailing newline. Provider stderr may be summarized on failure, but output
containing the requested value must never be echoed.

`pass` stores below the fixed `oslo/` prefix:

```text
pass insert --force --multiline oslo/<name>
pass show oslo/<name>
pass rm --force oslo/<name>
```

Write the stored value through stdin. Validate names before constructing the provider path.

### Scoped child execution

Do not temporarily mutate `std::env` or `Environment`. Extend the existing external spawn path with
an execution specification containing:

- resolved program path;
- argv;
- a copied base environment from `Environment::get_exported_vars()`;
- injected secret environment pairs;
- optional stdin bytes.

The forked child calls `execve` with the constructed environment. The parent never publishes the
secret. For stdin injection, create a pipe, give the read end to child fd 0, write the secret from the
parent, close the write end, then wait. Preserve the existing signal reset and wait-status behavior in
`env/builtins/spawn.rs`; do not create a second incompatible fork/wait implementation.

Reject NUL in environment names or values. Stdin injection remains byte-preserving.

## Current-state evidence

- `crates/oslo-runtime/src/startup/read.rs:307-367` derives a `secret` boolean only from leading
  whitespace and records command frecency before returning the accepted command.
- `crates/oslo-runtime/src/startup/repl.rs:238-263` gates flat history, recall and Tagdata on that
  boolean before execution.
- `crates/oslo-runtime/src/startup/repl.rs:283-317` hands raw text to hooks and title rendering;
  semantic metadata is already gated.
- `crates/oslo-runtime/src/startup/repl.rs:408-460` sends raw text to chain reporting,
  notifications, post-command hooks and tracking unless the leading-space flag was set.
- `crates/oslo-runtime/src/startup/notify.rs:40-66` can expand `{cmd}` into a notification title or
  a `/bin/sh -c` notification command.
- `crates/oslo-shell/src/exec/simple/trace.rs:18-26` prints expanded assignments and argv under
  `set -x` and currently has no private-command suppression.
- `crates/oslo-base/src/track/redact.rs:239-352` has useful credential heuristics, but they run when
  reducing tracking detail rather than before raw history persistence.
- `crates/oslo-ui/src/ask/input.rs:20-103` already supports masked input and returns the value without
  drawing it.
- `crates/oslo-shell/src/env/builtins/ui.rs:140-168` exposes that widget as `ui input --password`.
- `crates/oslo-shell/src/env/builtins/mod.rs:118-210` is the single default-builtin registry.
- `crates/oslo-shell/src/env/builtins/spawn.rs:46-84` is the existing fork, exec and wait path for an
  external program invoked by a builtin.
- `crates/oslo-shell/src/tab/keeper.rs:201-239` appends PTY output bytes to the tab log.
- `crates/oslo-shell/src/tab/log.rs:34-59` stores those raw bytes in the capped transcript.

## Scope

### In scope

- `crates/oslo-base/src/lib.rs`
- `crates/oslo-base/src/secret.rs` (new)
- `crates/oslo-base/src/track/redact.rs`
- `crates/oslo-runtime/src/startup/read.rs`
- `crates/oslo-runtime/src/startup/repl.rs`
- `crates/oslo-runtime/src/startup/notify.rs`
- `crates/oslo-runtime/src/startup/editor.rs`
- `crates/oslo-runtime/src/startup/prompt.rs`
- `crates/oslo-runtime/src/lua/engine.rs`
- `crates/oslo-shell/src/env/builtins/mod.rs`
- `crates/oslo-shell/src/env/builtins/spawn.rs`
- `crates/oslo-shell/src/env/builtins/secret.rs` and `secret/` children (new)
- `crates/oslo-shell/src/exec/mod.rs`
- `crates/oslo-shell/src/exec/simple/trace.rs`
- `crates/oslo-ui/src/settings/mod.rs`
- `crates/oslo-ui/src/settings/from_lua.rs`
- `crates/oslo-ui/src/settings/tests.rs`
- focused existing tests under `crates/oslo-base`, `crates/oslo-runtime` and `crates/oslo-shell`
- `tests/startup_tests.rs`
- `tests/terminal_semantics_tests.rs` and its existing child modules when metadata coverage is needed
- `docs/features/secrets.md` (new)
- `CHANGELOG.md` only when the implementation is release-ready

### Out of scope

- All files under `crates/oslo-shell/src/tab/` and the active tab refactor.
- `crates/oslo-ui/src/ask/choose.rs` and unrelated UI refactoring.
- Changing the tab log from raw bytes to a semantic transcript.
- Editing or expanding `README.md`; feature detail belongs in `docs/features/secrets.md`.
- Native encryption or Tagdata schema changes.
- Secret sync, sharing, rotation, leases or remote Vault support.
- macOS Keychain, Windows Credential Manager and clipboard integration.
- Output masking or rewriting arbitrary child output.
- Provider SDK dependencies.
- Changing non-interactive shell history behavior.

## Git workflow

- Start only from a worktree with no unrelated modifications in scope.
- Branch: `feat/secrets` from `develop`.
- Commits are title-only Conventional Commits, under 50 characters, with no signature or attribution.
- Recommended commits:
  1. `test(secret): characterize privacy sinks`
  2. `feat(secret): gate sensitive commands`
  3. `feat(secret): add provider storage`
  4. `feat(secret): inject child secrets`
  5. `docs(secret): document threat model`
- Do not push, merge or open a PR unless separately instructed.
- Code comments describe code behavior only. Do not put design justification or change history in code
  comments, and keep comments below 20% of implementation lines in each code block.

## Commands

Use the Makefile for every relevant repository task.

| Purpose | Command | Expected result |
|---------|---------|-----------------|
| Fast compile gate | `make check` | exit 0 |
| Terminal tests | `make test-terminal` | exit 0 |
| Full repository gate | `make verify` | exit 0 |
| Minimal release | `make build TYPE=minimal` | exit 0; static, no `INTERP`, no `NEEDED` |
| All-feature release | `make build` | exit 0; static, no `INTERP`, no `NEEDED` |

Record exact binary bytes with `stat -c %s target/x86_64-unknown-linux-musl/release/oslo` immediately
after each Makefile build. Do not compare the rounded megabyte line printed by Make.

## Implementation phases

### Phase 0: Record the baseline

1. Confirm the active tab refactor is no longer an uncommitted in-scope risk.
2. Run `make verify` before changes.
3. Run `make build TYPE=minimal` and record the exact byte count.
4. Run `make build` and record the exact byte count.
5. Record `git status --short` with no unrelated in-scope modifications.

**Verify**: both Makefile gates exit 0 and both byte counts are recorded in the implementation PR or
commit notes, not in a code comment.

### Phase 1: Characterize every current sink

Before changing behavior, add synthetic-marker tests proving where the existing leading-space flag
does and does not gate data. Cover:

- flat history file;
- in-memory `history` output;
- recall and history expansion;
- Tagdata raw history and its finder/query surface;
- command frecency;
- tracking aggregate and outcomes;
- semantic output-start metadata;
- prompt title context;
- pre-command and post-command Lua hook payloads;
- slow notification template and `on-report` payload;
- chain segment and resume text;
- `set -x` expanded output.

Every test uses a generated synthetic marker with a stable prefix such as `OSLO_TEST_SECRET_`; never
use a valid live token.

**Verify**: `make test-terminal && make test` both exit 0. The new tests must demonstrate the existing
gaps before production changes; mark gap assertions with the phase they are expected to pass after,
not with ignored tests that can remain forgotten.

### Phase 2: Add settings and the detector

1. Add `Secret` settings and defaults under `Settings` in `oslo-ui`.
2. Parse and validate `oslo.secret` in `settings/from_lua.rs`.
3. Add settings tests for defaults, both providers, disabled detection, notice, allowlists, sorted
   custom patterns, invalid IDs, invalid regexes and unknown providers.
4. Add `oslo-base::secret::{Sensitivity, Detector}`.
5. Split credential evidence from tracking-only minimization in `track/redact.rs`.
6. Add a table-driven detector suite for every rule, quoting forms, false-positive counterexamples,
   custom rules and allowlisted rules.

False-positive counterexamples must include:

- `openssl version`;
- `gpg --version`;
- a non-secret `FOO=bar command` assignment;
- a long path;
- a multiline ordinary shell construct;
- a dotted version string;
- a mixed-case build identifier without secret context.

**Verify**: `make check && make test` exit 0.

### Phase 3: Move classification before persistence

1. Change the accepted input type to carry `Sensitivity`.
2. Classify the completed expanded buffer before frecency recording.
3. Retain the original leading-space decision as `ExplicitPrivate`.
4. Replace every REPL `secret` boolean branch with the centralized sensitivity gate.
5. Reclassify pre-hook replacements without allowing a downgrade.
6. Add trace suppression for private top-level execution.
7. Make hook contexts redacted and add `sensitive` plus `secret_rule`.
8. Suppress private title, notification, report, chain and metadata text.
9. Print only the optional safe detection notice.

**Verify**: `make test-terminal && make test` exit 0. All Phase 1 synthetic-marker assertions now pass
for automatic detection and explicit leading-space privacy.

### Phase 4: Add provider storage and reveal UX

1. Register the `secret` builtin in the one default registry.
2. Implement one source of truth for top-level and subcommand help.
3. Implement strict argument parsing and `SecretName` validation.
4. Implement masked `put` and `edit` with the existing `ask::input` widget.
5. Implement the Secret Service adapter through `secret-tool`.
6. Implement the `pass` adapter.
7. Implement the securely created, atomically rewritten managed-name index.
8. Implement `list`, confirmed TTY-only `show`, confirmed `rm`, and `status`.
9. Use mock provider executables in tests. Assert the synthetic value is absent from argv and appears
   only on the provider stdin or captured lookup stdout boundary.

Provider failure tests cover missing executable, non-zero exit, closed stdin, invalid UTF-8 error
messages, missing name, stale index entry and unavailable Secret Service. Diagnostics name the
provider operation and secret name but never the value.

**Verify**: `make check && make test` exit 0.

### Phase 5: Add scoped child injection

1. Parse repeated `--env VAR=NAME`, optional `--stdin NAME`, mandatory `--`, and external argv.
2. Resolve every named secret before forking. If any lookup fails, run no child.
3. Extend the existing spawn path to accept an explicit child environment and optional stdin bytes.
4. Build the child environment from exported Oslo variables plus injected values without mutating the
   parent environment.
5. Use `execve`, preserve current signal reset and wait status, and close every unused pipe end.
6. Drop retrieved values immediately after spawn completion.

Tests must prove:

- the target sees each requested environment variable;
- the target receives exact stdin bytes and EOF;
- the target argv contains no secret value;
- the provider argv contains no secret value;
- the parent environment is unchanged before and after success, target failure and provider failure;
- duplicate target environment names are rejected rather than resolved by order;
- invalid environment names and NUL-containing environment values are rejected;
- the target never starts after a partial provider-lookup failure;
- exit codes and signal statuses match the existing spawn behavior.

**Verify**: `make test-terminal && make test` exit 0.

### Phase 6: Document the honest boundary

Create `docs/features/secrets.md` containing:

- automatic detection behavior and rule IDs;
- the full `oslo.secret` configuration;
- every builtin form with examples using fake names and values;
- provider setup requirements;
- the recommendation to prefer stdin over environment variables;
- the fact that Secret Service attributes and secret names are metadata;
- the ordinary-editor and raw-tab-transcript limitation;
- the fact that `show` and target output enter scrollback;
- the explicit statement that detection is a safety net, not a guarantee;
- recovery instructions for an accidentally entered real secret: rotate it, then remove it from any
  terminal or external logs; Oslo cannot prove deletion outside its own stores.

Do not expand `README.md` with the feature guide.

**Verify**: `make check-readme && make rustdoc` exit 0.

### Phase 7: Full security and size gate

1. Run `make verify`.
2. Run `make build TYPE=minimal`; record exact bytes and compare with Phase 0.
3. Run `make build`; record exact bytes and compare with Phase 0.
4. Confirm both binaries remain static through the Makefile's checks.
5. Inspect `git status --short` and reject any file outside Scope.
6. Search generated test artifacts and configured scratch stores for the synthetic marker. It may
   exist only in the mock provider's intentional secret storage and the target's intentional
   environment/stdin capture.

The minimal release binary may grow by at most **131,072 bytes**. If it exceeds that limit, do not
hide the increase by changing release flags. Remove duplication and unused surface, then measure
again. If it still exceeds the limit, stop and present the symbol/dependency evidence for a user
decision.

**Verify**: all Makefile gates exit 0; minimal size delta is at most 131,072 bytes; no new runtime
dependency appears; no out-of-scope file is modified.

## Test matrix

### Detector

- One positive and at least two negative cases per built-in rule.
- Quoted, unquoted, glued-option and separate-option forms.
- Shell and Lua interactive modes.
- History-expanded command classified after expansion.
- Leading-space privacy with detection both enabled and disabled.
- Custom pattern match, invalid pattern and allowed rule.
- Notice contains rule ID and never the matched substring.

### Persistence and observation

- `$HISTFILE` absent marker.
- `history` builtin absent marker.
- recall and history expansion cannot recover marker.
- Tagdata history/search/export/backup input absent marker.
- frecency and tracking absent marker.
- predictor receives no private line when the optional feature is enabled.
- pre/post hook payload text is `<private>` and sensitivity fields are present.
- title and semantic output-start omit marker.
- notification command and report hook are not invoked for private commands.
- chain recorder and `chain resume` retain no private line.
- xtrace prints no private assignments or argv.

### Builtin and providers

- Every listed subcommand has help and returns status 0 for help.
- Unknown options/subcommands return status 2 without provider access.
- Name grammar accepts nested names and rejects traversal/newlines.
- Masked entry handles accept, empty value, cancel and no terminal.
- Provider store receives value only on stdin.
- Provider lookup output is never included in an error.
- List reads metadata only.
- Show refuses redirected stdout and confirms on a TTY.
- Remove updates index only after provider success.
- Provider selection never falls back silently.

### Scoped execution

- Multiple environment injections.
- Exact stdin injection.
- Child status and signal propagation.
- No parent-environment mutation on every exit path.
- No provider or target argv leak.
- No shell-source interpolation.

### Known limitation test

Add or retain a test demonstrating that literal text typed in the ordinary editor may appear in the
raw tab transcript before Enter. Mark it as a documented limitation, not as a passing secrecy claim.
Separately prove that a value entered through masked `secret put` is absent from the editor and tab
transcript because it reaches the provider through an internal pipe.

## Done criteria

- [ ] `Sensitivity` replaces the interactive `secret: bool` gate.
- [ ] Classification runs before every Oslo persistence or observation sink.
- [ ] Explicit leading-space privacy still works.
- [ ] High-confidence automatic rules have positive and false-positive tests.
- [ ] Sensitive commands execute unchanged.
- [ ] Sensitive text is absent from history, recall, Tagdata, frecency, tracking and prediction.
- [ ] Sensitive text is absent from hooks, title, notification, metadata, chain and xtrace.
- [ ] `secret put/edit/list/show/rm/run/status` match the documented contract.
- [ ] Secret values never enter provider argv, target argv or parent exported environment.
- [ ] Secret Service and `pass` adapters use direct process execution.
- [ ] Tagdata contains neither secret values nor a secret vault.
- [ ] No new runtime dependency is added.
- [ ] `README.md` and tab source files are unchanged.
- [ ] `make verify` exits 0.
- [ ] `make test-terminal` exits 0.
- [ ] `make build TYPE=minimal` and `make build` produce static binaries.
- [ ] Minimal binary growth is at most 131,072 bytes.

## STOP conditions

Stop and report instead of improvising if:

- the current input or REPL flow no longer matches the Current-state evidence;
- an in-scope file contains unrelated uncommitted work;
- protecting a sink appears to require parsing or rewriting shell source;
- a provider requires passing a value in argv or through `/bin/sh -c`;
- provider listing cannot avoid reading values and the metadata index is rejected;
- scoped injection requires publishing the secret into the parent process environment;
- xtrace cannot be suppressed for one top-level interactive execution without changing script
  behavior globally;
- a test needs a real credential or network access;
- a new runtime dependency appears necessary;
- minimal binary growth remains above 131,072 bytes after removing duplication;
- tab transcript protection would require touching the active tab refactor;
- a verification gate fails twice after a reasonable correction.

## Review checklist

The reviewer must inspect these specifically:

- classification happens before `record_command_use`, history append and hooks;
- private taint survives pre-hook replacement;
- no diagnostic formats `SecretValue`;
- provider and child argv contain names only;
- parent environment is never temporarily mutated;
- pipe ends close on every success and failure path;
- provider stderr cannot echo a looked-up value;
- `secret show` is the only builtin path that intentionally prints a value;
- output masking has not been smuggled into pipeline bytes;
- the documented tab limitation remains explicit;
- no new dependency or unexplained binary growth appears.

## Maintenance notes

- New history, prediction, telemetry, hook or terminal-metadata sinks must accept `Sensitivity` rather
  than inventing a separate filter.
- New providers must implement the same no-shell, no-value-in-argv contract and be measured for
  binary impact.
- Provider names and lookup attributes remain public metadata even when values are encrypted.
- If tab logging later becomes semantic, input-span suppression should be designed in the tab plan;
  do not weaken this plan's limitation statement until a transcript test proves it.
- If output masking is reconsidered, it may affect terminal rendering only. It must never modify bytes
  delivered through pipes, files, command substitution or structured output.

## Research references

- Atuin secret filtering: <https://docs.atuin.sh/main/configuration/config/>
- detect-secrets detector architecture: <https://github.com/Yelp/detect-secrets>
- Gitleaks rules and allowlists: <https://github.com/gitleaks/gitleaks>
- 1Password scoped process injection: <https://developer.1password.com/docs/cli/reference/commands/run/>
- AWS Vault external storage and temporary credentials: <https://github.com/99designs/aws-vault>
- Secret Service specification: <https://specifications.freedesktop.org/secret-service/latest-single/>
- Secret Service lookup attributes are metadata:
  <https://specifications.freedesktop.org/secret-service/latest/lookup-attributes.html>
- `secret-tool` command contract: <https://man.archlinux.org/man/core/libsecret/secret-tool.1.en>
- `pass`: <https://www.passwordstore.org/>
- SOPS scoped execution patterns: <https://github.com/getsops/sops>
- Vault Agent and dynamic-secret direction:
  <https://developer.hashicorp.com/vault/docs/agent-and-proxy/agent>
