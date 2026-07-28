# mode: posix
f() { return 2; }
f; echo "$?"
f || echo "or=$?"
