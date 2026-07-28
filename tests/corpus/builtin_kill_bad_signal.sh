# mode: posix
kill -NOSUCHSIG $$ 2>/dev/null
echo "status=$?"
echo STILL_ALIVE
