# mode: posix
# Assignments in one command are evaluated left to right, each visible to the next.
a=1 b=${a}2
echo "$a $b"
c=x c=y
echo "$c"
