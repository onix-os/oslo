# mode: posix
# A match keeps the path syntax the pattern was written in: `./a*` matches `./a1`, not `a1`.
touch a1 a2
mkdir d
touch d/a1
echo ./a*
p=.
echo "$p"/a*
echo d/a*
