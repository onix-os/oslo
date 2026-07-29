# mode: posix
# POSIX: an unsigned-integer first operand means every operand is a condition to reset.
trap 'echo should_not_run' EXIT
trap 0
echo body
