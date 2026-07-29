# mode: posix
x=1
echo $((0 && (x = 9)))
echo "$x"
echo $((1 || (x = 9)))
echo "$x"
echo $((0 ? (x = 5) : 7))
echo "$x"
echo $((1 ? (x = 5) : 7))
echo "$x"
