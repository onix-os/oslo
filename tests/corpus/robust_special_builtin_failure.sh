# mode: posix
# POSIX 2.8.1: a *special* builtin that hits a utility error makes a non-interactive shell exit.
# `export` is one, so neither shell prints the sentinel.
#
# The rule is narrower than "non-zero status", which is why this case is worth having: `shift 5`
# also fails and is *not* fatal (see builtin_shift.sh), so the builtin has to say which kind of
# failure it had rather than just returning a number.
export "=1" 2>/dev/null
echo "status=$?"
echo STILL_ALIVE
