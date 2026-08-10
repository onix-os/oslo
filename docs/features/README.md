# Features

One document per feature: what it is, how it works, what it cannot do, and where it lives in the
tree. These go deeper than the README — the README says a feature exists, these say how it is built
and why it is built that way.

Every claim in here was checked against the source. Where a design was forced by a measurement or by
a bug, the document says so, because those are the sentences worth reading twice.

## The two languages

| | |
|---|---|
| [Two languages, one prompt](two-languages-one-prompt.md) | Shell and Lua at the same prompt, switched in place |
| [The Lua interpreter](lua-interpreter.md) | Lua in pure Rust — what lets a static musl binary speak it with no C toolchain |
| [Your own tools](your-own-tools.md) | `register_tool`, builtins and autoloaded functions from Lua |
| [Hooks](hooks.md) | The points a config can attach to, and what a return value means |
| [Drawing](drawing.md) | The shell's own output widgets, and taking them over |

## The pipeline

| | |
|---|---|
| [Structured pipelines](structured-pipelines.md) | Rows for the next stage, text for you — decided before anything runs |
| [POSIX, where it counts](posix-fidelity.md) | What a script is guaranteed, and the corpus that proves it |

## Typing

| | |
|---|---|
| [The line editor](line-editor.md) | oslo owns the row it edits: buffer, layout, redraw, keymaps |
| [Ghost suggestions](ghost-suggestions.md) | The grey continuation, and the four sources you order yourself |
| [Prediction and repair](prediction-and-repair.md) | A model of what you run: what comes next, and what you meant |
| [Completion and matching](completion-and-matching.md) | The dropdown, and matching as a transform rather than a prefix test |
| [Abbreviations](abbreviations.md) | `gco ` becomes `git checkout ` in the buffer, where you can see it |

## Memory

| | |
|---|---|
| [What gets written down](what-gets-written-down.md) | The log, the outcomes, the chains, and what is deliberately not recorded |
| [The history finder](history-finder.md) | Full-screen search with scopes that narrow and widen |
| [Where you have been](where-you-have-been.md) | Directory tracking, `cd -N`, `cd root`, and ranking that puts match quality first |
| [One shell, several histories](profiles-and-histories.md) | `$OSLO_PROFILE`, and keeping an agent's commands out of yours |

## The environment

| | |
|---|---|
| [Directory environments](directory-environments.md) | `.envrc` read by oslo itself, not handed to direnv |
| [The filesystem navigator](nav.md) | `nav`: type to filter, arrows to move, Esc to take the shell there |
| [rm, and the things that can bite](rm-and-safety.md) | Recoverable at the prompt, POSIX in a script |

## Appearance and control

| | |
|---|---|
| [The prompt](the-prompt.md) | Named segments with priorities, gathered once |
| [Colours](theme.md) | Every role settable, with inheritance and background detection |
| [The terminal knows what is happening](terminal-integration.md) | What oslo tells the terminal and the multiplexer |
| [Features you can turn off](runtime-features.md) | A runtime mask over your configuration, never an assignment to it |

## Reading these

Each document has the same shape:

```
How it works          the mechanism, with a diagram
What makes it         the contrast with bash, zsh or fish — stated only where it
  different             could be checked
Configuration         spellings verified against the code that reads them
Measurements          real numbers only; the section is absent when there are none
What it cannot do     required, and never empty
Where it lives        paths and the types or functions that matter
```

The **What it cannot do** section is the one to read first if you are deciding whether to rely on
something. It is required in every document precisely because a feature list that only lists wins is
not documentation.
