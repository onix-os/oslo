# mode: posix
umask 022
umask 999 2>/dev/null
echo "status=$?"
umask
