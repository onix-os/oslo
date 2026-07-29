# mode: bash
# The other half of `builtin_readonly.sh`, which runs in POSIX mode and therefore only ever sees
# the shell give up. Outside POSIX mode a refused assignment is an ordinary failed command: the
# diagnostic goes to stderr, `$?` is 1, the old value survives, and the script carries on. The
# status is the whole point — it used to be 0, so a script guarding `r=$new || die` never fired.
readonly r=1
r=2
echo "assigned=$?"
echo "value=$r"
r+=tail
echo "appended=$?"
echo "value=$r"
echo after
