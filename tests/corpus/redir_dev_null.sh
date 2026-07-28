# mode: posix
echo swallowed > /dev/null
echo "$?"
cat /dev/null
echo empty_ok
