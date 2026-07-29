# mode: posix
# The same shape as builtin_exec_redirect.sh on a descriptor that does not collide with the one
# the shell's own open() lands on, so it tests `exec`'s permanence and nothing else.
exec 4> out.txt
echo written >&4
exec 4>&-
cat out.txt
