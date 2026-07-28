# mode: posix
set -- a b
echo "$#"
echo "$0" | grep -c .
false
echo "$?"
