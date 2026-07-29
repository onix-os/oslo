# mode: posix
# With nothing outstanding, `jobs` prints nothing and succeeds. A script that polls
# for background work runs this on every iteration.
jobs
echo "empty=$?"
sh -c 'exit 0' &
wait
jobs
echo "waited=$?"
