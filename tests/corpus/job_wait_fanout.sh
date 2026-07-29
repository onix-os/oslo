# mode: posix
# The fan-out/fan-in pattern, and the hardest case for `wait`: by the time the second
# loop runs, the shell's opportunistic reaper has already collected most of these
# children, so every status here has to come from the job table rather than the kernel.
i=0
while [ $i -lt 5 ]; do
  sh -c "exit $i" &
  eval "p$i=\$!"
  i=$((i + 1))
done
i=0
while [ $i -lt 5 ]; do
  eval "pid=\$p$i"
  wait "$pid"
  echo "$i=>$?"
  i=$((i + 1))
done
wait
echo "rest=$?"
