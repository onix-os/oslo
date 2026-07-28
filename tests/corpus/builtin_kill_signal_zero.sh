# mode: posix
# kill -0 probes, it must not signal.
kill -0 $$
echo "probe=$?"
echo STILL_ALIVE
