# mode: bash
# The `hash` table describes the *session*, not just what an explicit `hash name` put in it: every
# bare command word resolved through `PATH` is remembered, and the hit count is what makes that
# observable. Until R11 wired command resolution through the table, `ls; hash` printed
# "hash table empty" here while bash listed `ls`.
#
# `ls` is used rather than a temporary script so that both shells resolve the same absolute path
# through the same `PATH`; the path is machine-specific but identical on both sides.
ls >/dev/null
hash
ls >/dev/null
# The second run is a cache hit, so the count is 2 — a table that were rebuilt each time would
# still say 1.
hash

# `hash name` re-seeds the entry and resets its count, so this prints 0 rather than 3.
hash ls
hash

# A builtin never enters the table, and neither does a word with a slash: `./x` means something
# different in every directory.
hash -r
true
/bin/sh -c ':'
hash
