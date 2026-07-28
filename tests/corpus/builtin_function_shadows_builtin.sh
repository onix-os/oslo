# mode: posix
# A function must take precedence over a non-special builtin.
cd() { echo "cd called with $1"; }
cd /tmp
unset -f cd
echo done
