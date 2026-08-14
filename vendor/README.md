# Vendored parsers

**Everything under `vendor/` is somebody else's code**, kept close to upstream and deliberately not
restyled — which is what the blanket lint allowances at the top of these crates rest on. oslo's own
crates live in `crates/`. The two directories used to be one, and the distinction was worth more
than the shorter path: a rule that applies to half a directory is a rule nobody can apply.

Two parsers oslo does not write and does not want to depend on remotely. `full_moon` carries its
proc-macro in `full_moon/derive`: a `proc-macro = true` crate can only export macros, so it cannot
live *inside* another crate, so nesting the directory is as close to one unit as cargo permits.

`brush-parser` and `full_moon` are hard forks: the source is here, oslo builds it as a workspace
member, and there is no upstream to sync with.

**`argc` is not.** It is actively developed, and oslo tracks it: this is 1.24.0, and a later release
is a rebase rather than a fork's divergence. What that costs is written down — the modifications are
few and listed below, so a rebase is a readable diff rather than an archaeology exercise:

| what | why |
|---|---|
| `src/bin/` removed, with the `application` feature and four dependencies only it used | oslo is the application |
| `#![allow(…)]` at the top of `lib.rs` | oslo lints at `-D warnings`; what is unused is unused only because oslo builds a subset of the features |
| `pub use anyhow;` | so a caller implementing `Runtime` can name `anyhow::Result` without depending on the crate for one type |

**oslo is MIT. `full_moon` and `full_moon/derive` are MPL-2.0 and remain so.** See *Licences* below
before copying anything out of this directory.

| crate | upstream | licence | why it is here |
|---|---|---|---|
| `argc` | [sigoden/argc](https://github.com/sigoden/argc) | MIT OR Apache-2.0 | the `# @option` declaration language, behind the `argc` feature |
| `brush-parser` | [reubeno/brush](https://github.com/reubeno/brush) | MIT | POSIX/bash tokenizer and parser |
| `full_moon` | [Kampfkarren/full-moon](https://github.com/Kampfkarren/full-moon) | **MPL-2.0** | Lua parser |
|`full_moon/derive` | as above | **MPL-2.0** | proc-macro `full_moon` needs; not published as a standalone path dep |

## Not here

`vista` — the prediction and repair model — was copied in here for one release cycle and is now an
ordinary git dependency of `oslo-base`, pinned to a commit. It was never a fork: it is oslo's own
crate, developed in its own repository and still moving. The copy existed only because the version
of it oslo needed declared an MSRV higher than oslo's own, which a `git` dependency would have
imposed on everyone building the shell. That was fixed upstream, so the copy had nothing left to
justify it.

The rule it illustrates is the one this directory runs on: **something is vendored because it is
forked, not because it needs pinning.** A revision pins a dependency perfectly well.

## Why vendored

Not to change what they parse. To decide what they *depend on*.

Between them these two crates were pulling 104 crates into oslo's build, 26 of which were
proc-macros — each its own dylib to compile and link before oslo's own code can start. Almost none
of it reached the binary. `cargo build` was paying for code that dead-code elimination then threw
away.

After trimming: **65 crates, 9 proc-macros, and one version of `syn` instead of three.**

## What was removed from `brush-parser`

Three non-optional dependencies, none of which survived into the linked binary:

* **`cached`** — 31 crates, including `parking_lot`, `ahash`, `hashbrown`, `zerocopy`, `web-time`,
  and its own copies of `darling` and `syn`. It memoised three functions. Replaced by
  `src/memo.rs`, which is sixty lines and does the same job at the same bound.
* **`bon`** — 13 crates, including a second `darling` and `prettyplease`. It generated
  `Parser::builder()`, which **nothing called** — not oslo, not brush-parser itself. The builder is
  deleted; `Parser::new` is the constructor and always was.
* **`tracing`** — 8 crates. Eight `tracing::debug!` calls, none reachable without a subscriber that
  oslo never installs.
* **`thiserror`** — now optional, and reached only by the `diagnostics` feature, where `miette`
  genuinely needs it. Three enums and two newtypes, twenty-eight messages, written out by hand in
  `error.rs` and `tokenizer.rs`. It was the last thing wanting `syn 3`, so the build now compiles
  exactly one version of `syn`. Every diagnostic was compared against the previous binary character
  for character before the derive came out: the wording is what a user sees when a script will not
  parse, and it has not changed.

Also removed: the `winnow-parser` feature and `parser/winnow_str.rs`, a twenty-line stub for an
alternative parser that was never finished and never enabled.

Everything else is upstream's, unmodified.

## What was removed from `full_moon/derive`

`indexmap`, which nothing in the crate referenced — a dead entry in the manifest dragging
`hashbrown 0.12` behind it. And `syn 1` to `syn 2`, which needed one function rewritten:
`search_hint`, whose `parse_meta`/`NestedMeta` were removed in syn 2 in favour of a
`parse_nested_meta` callback.

**The crate itself stays.** It generates `Node` and `Visit` for 57 and 44 types; replacing it means
writing several thousand lines of mechanical recursion to do what 926 lines of generator already
does. That is copying out a macro's output by hand, not removing a dependency.

## What was removed from `full_moon`

Its `serde` default feature, which oslo does not use — that alone took `serde_derive` and one of
the three `syn` versions out of the build. Two dead private helpers left behind by it. The lint
allow at the top of `src/lib.rs` is there because oslo builds with `-D warnings` and **vendored
code is not restyled**: a diff full of house-style edits is what makes a vendored crate impossible
to read against upstream later.

## Licences

`brush-parser` is MIT, © 2024 reuben olinsky. `LICENSE` beside it is upstream's, unchanged.

`full_moon` and `full_moon/derive` are **MPL-2.0**, © the full-moon authors. MPL is file-level
copyleft: those files stay MPL however they are combined, and any modification to them must remain
available in source form. That is compatible with oslo being MIT — MPL is designed for exactly this
— but it is a real obligation and it does not evaporate because the code sits in this tree. The
crates ship no `LICENSE` on crates.io; the copies here were added to satisfy the notice
requirement.

If you take code out of this directory, take the licence with it.
