# mode: posix
# POSIX: a *special* builtin that fails makes a non-interactive shell exit. `export` is one, so
# `bash --posix` stops here and never prints the sentinel. rush has no notion of special builtins
# and carries on, which is plain-bash behaviour rather than POSIX behaviour.
export "=1" 2>/dev/null
echo "status=$?"
echo STILL_ALIVE
