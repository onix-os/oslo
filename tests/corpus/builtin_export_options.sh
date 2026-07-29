# mode: posix
export XA=1
export -p | grep '^export XA=' > /dev/null
echo "listed=$?"
export -p > /dev/null
echo "after=${XA}"
