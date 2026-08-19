#!/usr/bin/env bash
# Fail if the README names a repository path that does not exist (PLAN R10.5).
#
# README drift is the cheapest documentation bug to introduce and the most expensive to notice:
# the audit that produced PLAN.md found the README advertising `tests/shell_behavior_tests.rs`,
# a file deleted long before, alongside a "Known gaps" list where most entries had already been
# fixed. Prose claims about behaviour cannot be checked mechanically, but claims about *paths*
# can be, and a stale path is a reliable tell that the surrounding paragraph is stale too.
#
# Scope is deliberately narrow, because a check that cries wolf gets deleted. Only backtick-quoted
# tokens rooted at one of this repository's own top-level directories are examined; `~/.oslorc`,
# `/dev/fd/N`, shell one-liners and module names cited as naming examples are all prose about
# something other than this tree, and are left alone.
set -euo pipefail

# `crates` is on this list because the code moved there and this check did not follow it. For a
# while afterwards every path under `crates/` was invisible to the only thing looking, and the
# feature pages accumulated exactly the drift the check exists to catch — one of them named
# `direnv/devshell.rs` for a file the split had renamed.
ROOTS='crates|src|tests|scripts|examples|fuzz|bench|vendor|.github|docs'

# Every page, not only the README: the feature pages cite far more paths than it does, and they are
# where a rename goes unnoticed longest.
PAGES=("${@:-README.md}")
if [ "$#" -eq 0 ]; then
    while IFS= read -r page; do PAGES+=("$page"); done < <(find docs -name '*.md' | sort)
fi

status=0

check() {
    local doc="$1" token probe
    while IFS= read -r token; do
        # Only paths under one of our own directories are claims about this tree.
        [[ "$token" =~ ^($ROOTS)/ ]] || continue
        # A backticked span containing whitespace is a command line, not a path.
        [[ "$token" =~ [[:space:]] ]] && continue
        # Trailing prose punctuation is not part of the path.
        token="${token%,}"
        token="${token%.}"
        # `src/env/` and `tests/corpus/*.sh` both name a directory that must exist.
        probe="${token%%\**}"
        # `ask/{input,write}.rs` names several files at once; the directory is what can be checked.
        probe="${probe%%\{*}"
        probe="${probe%/}"
        [ -z "$probe" ] && continue
        if [ ! -e "$probe" ]; then
            printf '%s: names `%s`, which does not exist\n' "$doc" "$token" >&2
            status=1
        fi
    done < <(grep -o '`[^`]*`' "$doc" | tr -d '`')
}

for page in "${PAGES[@]}"; do
    [ -e "$page" ] && check "$page"
done

if [ "$status" -ne 0 ]; then
    echo >&2
    echo "A page names files this tree does not have. Either the file was renamed or" >&2
    echo "deleted and the page was not updated, or the path is a typo." >&2
fi

exit "$status"
