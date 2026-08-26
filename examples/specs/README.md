# Completion specs

A `.yaml` per command, in [carapace-spec](https://github.com/carapace-sh/carapace-spec) format.
Copy one to where oslo looks and the command completes:

```sh
mkdir -p ~/.config/oslo/specs
cp deploy.yaml ~/.config/oslo/specs/
```

oslo reads, in order, `$OSLO_SPECS` (a colon list), `~/.config/oslo/specs`, and
`~/.config/carapace/specs` — so a machine that already has carapace specs keeps them. The **file
name** is the command it answers for. Nothing is read until that command is completed, and a spec
that is not there is remembered as not there.

Requires the `spec` cargo feature, which the release build has.
→ [docs/features/completion-and-matching.md](../../docs/features/completion-and-matching.md)

Specs can be generated rather than written: [carapace-spec-clap] for any clap program,
[carapace-spec-man] from manpages, and one each for kong, kingpin, urfave/cli, oclif and click.

[carapace-spec-clap]: https://github.com/carapace-sh/carapace-spec-clap
[carapace-spec-man]: https://github.com/carapace-sh/carapace-spec-man
