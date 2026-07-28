# mode: posix
echo $((1 && 0))
echo $((1 && 2))
echo $((0 || 0))
echo $((0 || 5))
echo $((!0))
echo $((!7))
