# tldr — examples where you are already looking

Offers what people actually do with a command, in the dropdown and as a ghost. The worked example
for both provider surfaces, and for the case where a plugin *adds* to what oslo already knows.

```sh
oslo plugin install examples/plugins/tldr
tldr git                 # what it knows
git c<TAB>               # `commit` from oslo's own spec, `commit --amend` from here
git commit --a           # the ghost completes it
```

It answers from `oslo.db`, so it is fast enough to be synchronous — the easy half. The
`score_offset = -5` is the interesting line: an example is worth reading when nothing better
matched, so it sits *below* a subcommand you actually run rather than above it.

Its pages are seeded rather than fetched, so this stays a plugin instead of a downloader.
