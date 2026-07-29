# mode: posix
# A pid the shell never started is 127, not 0 — and it says so on stderr.
wait 999999
echo "unknown=$?"
sh -c 'exit 6' &
p=$!
wait "$p"
echo "first=$?"
wait "$p"
echo "again=$?"
