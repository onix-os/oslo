# Vendored code

**Everything under `vendor/` is somebody else's code**, kept close to upstream and deliberately not
restyled — which is what the blanket lint allowances at the top of these crates rest on. oslo's own
crates live in `crates/`. The two directories used to be one, and the distinction was worth more
than the shorter path: a rule that applies to half a directory is a rule nobody can apply.

**There is one crate left here, and it is not a fork.** `argc` is actively developed and oslo tracks
it: this is 1.24.0, and a later release is a rebase rather than a fork's divergence. What that costs
is written down — the modifications are few and listed here, so a rebase is a readable diff rather
than an archaeology exercise:

| what | why |
|---|---|
| `src/bin/` removed, with the `application` feature and four dependencies only it used | oslo is the application |
| `#![allow(…)]` at the top of `lib.rs` | oslo lints at `-D warnings`; what is unused is unused only because oslo builds a subset of the features |
| `pub use anyhow;` | so a caller implementing `Runtime` can name `anyhow::Result` without depending on the crate for one type |

| crate | upstream | licence | why it is here |
|---|---|---|---|
| `argc` | [sigoden/argc](https://github.com/sigoden/argc) | MIT OR Apache-2.0 | the `# @option` declaration language, behind the `argc` feature |

## Not here

Three crates oslo depends on are **git dependencies pinned to a tag**, not copies:

| crate | where | what it is |
|---|---|---|
| `luna` | [onix-os/luna](https://github.com/onix-os/luna) | the Lua VM, oslo's own |
| `rune` | [onix-os/rune](https://github.com/onix-os/rune) | the shell parser, oslo's own |
| `vista` | [bresilla/vista](https://github.com/bresilla/vista) | the prediction and repair model, oslo's own |

`vista` was copied in here for one release cycle and is now an ordinary git dependency of
`oslo-base`. It was never a fork: it is oslo's own crate, developed in its own repository and still
moving. The copy existed only because the version oslo needed declared an MSRV higher than oslo's
own, which a `git` dependency would have imposed on everyone building the shell. That was fixed
upstream, so the copy had nothing left to justify it.

The rule it illustrates is the one this directory runs on: **something is vendored because it is
forked, not because it needs pinning.** A tag pins a dependency perfectly well.

## The two parsers that used to be here

Both were forks of other people's code, and both were replaced by oslo's own.

**`full_moon`** parsed Lua and was **MPL-2.0**, which is file-level copyleft: those files stayed MPL
however they were combined, and any modification had to remain available in source form. That is
compatible with oslo being MIT — MPL is designed for exactly this — but it was a real obligation
carried in the tree. `luna` replaced it, and the obligation went with it.

**`brush-parser`** parsed POSIX shell and bash. `rune` replaced it, and
`crates/oslo-shell/src/syntax/rune_adapter/` lowers rune's tree into oslo's AST.

Their being here was never about changing what they parsed. It was about deciding what they
*depended on*: between them they pulled 104 crates into oslo's build, 26 of them proc-macros, each
its own dylib to compile and link before oslo's own code could start. Almost none of it reached the
binary — `cargo build` was paying for code that dead-code elimination then threw away, and the
trimming got it to 65 crates and 9 proc-macros. Owning both parsers outright is the same argument
carried to its end: nothing to trim, because nothing arrived unasked for.

## Licences

`argc` is MIT OR Apache-2.0. If you take code out of this directory, take the licence with it.
