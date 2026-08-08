#!/usr/bin/env bash
# Enforce the project's file-length limit.
#
# No source file may exceed MAX_LOC lines. The limit exists so that every file stays small
# enough to hold in your head at once; when one grows past it, that is a signal it has taken on
# more than one responsibility and wants splitting along that seam.
#
# Split files must be named for what they contain — `redirects.rs`, `quoting.rs` — never
# `part1.rs` or `helpers2.rs`.
set -euo pipefail

MAX_LOC="${MAX_LOC:-600}"
status=0

while read -r count file; do
    [ "$file" = "total" ] && continue
    if [ "$count" -gt "$MAX_LOC" ]; then
        printf '%s: %s lines (limit %s)\n' "$file" "$count" "$MAX_LOC" >&2
        status=1
    fi
# `crates` is oslo's own code and is held to the limit like `src`. `vendor` is not: it is somebody
# else's, kept close to upstream and deliberately not restyled — see `vendor/README.md`.
#
# Naming the directories rather than scanning the repository is what keeps that distinction, and
# adding `crates` here is not optional bookkeeping: the moment code moves out of `src/` the rule
# silently stops applying to it, and a limit that quietly stops being checked is worse than no
# limit at all.
done < <(find src crates tests examples -name '*.rs' -type f -print0 2>/dev/null | xargs -0 wc -l | sort -rn)

if [ "$status" -ne 0 ]; then
    echo >&2
    echo "Files above the $MAX_LOC-line limit. Split them into modules named for their" >&2
    echo "contents, not their order." >&2
fi

exit "$status"
