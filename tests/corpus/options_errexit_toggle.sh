# mode: posix
# `set +e` puts it back, and the option is visible in `$-` while it is on.
set -e
case "$-" in *e*) echo "errexit in flags" ;; *) echo "MISSING" ;; esac
set +e
false
echo "survived with $?"
case "$-" in *e*) echo "STILL THERE" ;; *) echo "errexit cleared" ;; esac
set -e
false
echo NOT_REACHED
