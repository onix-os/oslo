# mode: posix
# With nothing outstanding, `jobs` prints nothing and succeeds. A script that polls
# for background work runs this on every iteration.
#
# The job sleeps before it exits, and that is load-bearing rather than decoration. bash announces
# a finished job it has not yet reported — `[1]+  Done  …` — the first time anything looks at the
# table, and whether it has already reported it depends on whether the child died before the shell
# reached `wait`. On a loaded runner it does, so this case printed the Done line under bash and
# nothing under oslo on some CI runs and matched on others, from the same commit. A job still
# running when `wait` claims it is announced there and never by the `jobs` that follows, which is
# the same assertion with the race taken out.
jobs
echo "empty=$?"
sh -c 'sleep 0.2; exit 0' &
wait
jobs
echo "waited=$?"
