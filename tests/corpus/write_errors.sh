# mode: posix
# A builtin that cannot write has failed, and the script needs to be told. These all exited 0
# before, because the write result was discarded. modernish calls the gap BUG_PUTIOERR and warns
# that it leaves a process feeding a pipe with no way to learn its reader has gone.
printf x > /dev/full
echo "printf=$?"

echo x > /dev/full
echo "echo=$?"

printf '%s\n' one two > /dev/full
echo "printf-multi=$?"

# The bytes must not survive the failure. They used to sit in the runtime's buffered stdout and get
# flushed at exit, by which time the shell had restored the descriptor — so `printf x > /dev/full`
# printed `x` on the terminal.
printf leaked > /dev/full 2>/dev/null
echo "nothing-leaked"

# A successful write is still 0, and still goes where it was sent.
printf ok > /tmp/oslo_write_ok.$$
echo "success=$? content=[$(cat /tmp/oslo_write_ok.$$)]"
rm -f /tmp/oslo_write_ok.$$
