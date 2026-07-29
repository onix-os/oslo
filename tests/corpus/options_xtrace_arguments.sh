# mode: posix
# The trace goes to stderr, so turning it on must not change what the script prints.
set -x
v=hello
echo "$v" "a b" ""
set +x
echo end
