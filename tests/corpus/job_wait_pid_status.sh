# mode: posix
# R7.5: the fan-out/fan-in idiom. Each `wait PID` reports that child's own status,
# and the bare `wait` that follows collects whatever is left and is always 0.
sh -c 'exit 3' &
a=$!
sh -c 'exit 4' &
b=$!
wait "$a"
echo "a=$?"
wait "$b"
echo "b=$?"
sh -c 'exit 5' &
wait
echo "all=$?"
