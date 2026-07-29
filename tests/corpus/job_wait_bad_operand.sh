# mode: posix
# An operand that is not a pid or a job spec is a usage error (1), and the operands
# around it are still waited for: the status is the last one's.
wait abc
echo "bad=$?"
sh -c 'exit 5' &
wait abc $!
echo "trailing=$?"
sh -c 'exit 5' &
wait $! abc
echo "leading=$?"
