# mode: posix
# `while`/`until` conditions are exempt, including the failing test that ends the loop.
set -e
i=0
while [ "$i" -lt 2 ]; do i=$((i + 1)); done
echo "while ended at $i"
until [ "$i" -ge 2 ]; do i=$((i + 1)); done
echo "until ended at $i"
while false; do echo NO; done
echo survived
