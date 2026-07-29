# mode: posix
# R7.4: a shell that never waits for its background jobs accumulates one zombie per job for the
# rest of its life. bash reaps on SIGCHLD, oslo reaps at command boundaries; either way, by the
# time a later command looks, there is nothing left in `Z` state.
i=0
while [ $i -lt 6 ]; do
  sleep 0 &
  i=$((i + 1))
done
sleep 0.4
:
count=$(ps -o stat= --ppid $$ | grep -c Z)
echo "zombies=$count"
