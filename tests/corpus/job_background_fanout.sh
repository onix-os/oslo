# mode: posix
# R7.2/R7.4: reaping background jobs opportunistically must not lose one. Every worker's output
# has to be on disk by the time `wait` returns, and `$!` has to still name the last job started
# even after the reaper has been round the table several times.
for n in 1 2 3 4 5; do
  { echo "worker $n" > "out.$n"; } &
done
wait
cat out.1 out.2 out.3 out.4 out.5
echo "last=$?"
sleep 0 &
last=$!
wait
test -n "$last" && echo "bgpid recorded"
