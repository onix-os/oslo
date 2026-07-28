# mode: posix
echo $((1 ? 2 : 3))
echo $((0 ? 2 : 3))
echo $((1, 2, 3))
