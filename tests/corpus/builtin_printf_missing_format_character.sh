# mode: posix
# A format that ends before its conversion letter is an error, not a literal percent: `%z` is a
# length modifier still waiting for one. It used to print `%` and report success.
printf '%z' 1
echo "z=$?"
printf 'a%'
echo "trailing=$?"
printf 'a%%b\n'
echo "escaped=$?"
