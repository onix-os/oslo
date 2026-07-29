# mode: posix
# Whatever the shell ignores for its own protection (SIGPIPE, and SIGTSTP/SIGTTIN/SIGTTOU in the
# REPL) has to be back at SIG_DFL in the process it execs — an ignored disposition survives exec,
# and a command that inherits an ignored SIGTSTP cannot be suspended at all.
grep SigIgn /proc/self/status
echo "status=$?"
