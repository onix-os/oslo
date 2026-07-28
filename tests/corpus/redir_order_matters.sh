# mode: posix
# 2>&1 before > file leaves stderr on the original stdout.
{ echo err >&2; } 2>&1 > file.txt
echo "file:"
cat file.txt
