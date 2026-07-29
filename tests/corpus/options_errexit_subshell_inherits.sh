# mode: posix
# errexit itself is inherited by a subshell, so the subshell stops at its own first failure.
set -e
(false; echo NOT_IN_SUB) || echo "sub died with $?"
echo survived
