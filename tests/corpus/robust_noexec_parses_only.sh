# mode: posix
# `set -n` reads the program and runs none of it. This was listed by `set -o` and did nothing, so
# `sh -n script` — how packaging validates maintainer scripts — executed them instead of checking
# them. A corpus case rather than a unit test because the whole point is what the *process* does.
echo before
set -n
echo SHOULD_NOT_RUN
