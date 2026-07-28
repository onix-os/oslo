# mode: posix
f() { return 3; }
f
echo "$?"
g() { echo before; return 0; echo after; }
g
echo "$?"
