# mode: posix
sh -c 'exit 7' &
wait $!
echo "$?"
