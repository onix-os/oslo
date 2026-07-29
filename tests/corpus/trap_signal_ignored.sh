# mode: posix
# `trap '' SIG` discards the signal; without it SIGTERM would end the shell.
trap '' TERM
kill -TERM $$
echo survived
