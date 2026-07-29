# mode: posix
# A background child killed by a signal is 128 + signo to `wait`, as it is to `$?`.
sh -c 'kill -TERM $$' &
wait $!
echo "term=$?"
