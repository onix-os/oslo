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
# tokens rooted at one of this repository's own top-level directories are examined; `~/.rushrc`,
# `/dev/fd/N`, shell one-liners and module names cited as naming examples are all prose about
# something other than this tree, and are left alone.
set -euo pipefail

README="${1:-README.md}"
ROOTS='src|tests|scripts|examples|fuzz|benches|vendor|.github'

status=0

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
    probe="${probe%/}"
    [ -z "$probe" ] && continue
    if [ ! -e "$probe" ]; then
        printf '%s: names `%s`, which does not exist\n' "$README" "$token" >&2
        status=1
    fi
done < <(grep -o '`[^`]*`' "$README" | tr -d '`')

if [ "$status" -ne 0 ]; then
    echo >&2
    echo "The README names files this tree does not have. Either the file was renamed or" >&2
    echo "deleted and the README was not updated, or the path is a typo." >&2
fi

exit "$status"
