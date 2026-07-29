# mode: posix
# `wait -n` reports the first job to finish. With no children left it is 127.
sh -c 'exit 6' &
wait -n
echo "one=$?"
wait -n
echo "none=$?"
