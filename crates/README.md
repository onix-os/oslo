# Vendored parsers

Two parsers oslo does not write and does not want to depend on remotely. Both are hard forks: the
source is here, oslo builds it as a workspace member, and there is no upstream to sync with.

**oslo is MIT. `full_moon` and `full_moon_derive` are MPL-2.0 and remain so.** See *Licences* below
before copying anything out of this directory.

| crate | upstream | licence | why it is here |
|---|---|---|---|
| `brush-parser` | [reubeno/brush](https://github.com/reubeno/brush) | MIT | POSIX/bash tokenizer and parser |
| `full_moon` | [Kampfkarren/full-moon](https://github.com/Kampfkarren/full-moon) | **MPL-2.0** | Lua parser |
| `full_moon_derive` | as above | **MPL-2.0** | proc-macro `full_moon` needs; not published as a standalone path dep |

## Why vendored

Not to change what they parse. To decide what they *depend on*.

Between them these two crates were pulling 104 crates into oslo's build, 26 of which were
proc-macros — each its own dylib to compile and link before oslo's own code can start. Almost none
of it reached the binary. `cargo build` was paying for code that dead-code elimination then threw
away.

After trimming: **72 crates, 13 proc-macros.**

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

Also removed: the `winnow-parser` feature and `parser/winnow_str.rs`, a twenty-line stub for an
alternative parser that was never finished and never enabled.

Everything else is upstream's, unmodified.

## What was removed from `full_moon`

Its `serde` default feature, which oslo does not use — that alone took `serde_derive` and one of
the three `syn` versions out of the build. Two dead private helpers left behind by it. The lint
allow at the top of `src/lib.rs` is there because oslo builds with `-D warnings` and **vendored
code is not restyled**: a diff full of house-style edits is what makes a vendored crate impossible
to read against upstream later.

## Licences

`brush-parser` is MIT, © 2024 reuben olinsky. `LICENSE` beside it is upstream's, unchanged.

`full_moon` and `full_moon_derive` are **MPL-2.0**, © the full-moon authors. MPL is file-level
copyleft: those files stay MPL however they are combined, and any modification to them must remain
available in source form. That is compatible with oslo being MIT — MPL is designed for exactly this
— but it is a real obligation and it does not evaporate because the code sits in this tree. The
crates ship no `LICENSE` on crates.io; the copies here were added to satisfy the notice
requirement.

If you take code out of this directory, take the licence with it.
