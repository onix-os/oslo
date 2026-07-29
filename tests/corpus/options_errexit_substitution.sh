# mode: posix
# An assignment reports the status of the substitution it ran, so `x=$(false)` is a failure.
set -e
x=$(echo captured)
echo "$x"
y=$(false)
echo NOT_REACHED
