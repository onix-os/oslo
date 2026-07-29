# mode: posix
x=5
echo $((x <<= 2))
echo "$x"
echo $((x >>= 1))
echo "$x"
echo $((x |= 3))
echo "$x"
echo $((x &= 6))
echo "$x"
echo $((x ^= 5))
echo "$x"
echo $((x -= 4))
echo "$x"
echo $((x /= 2))
echo "$x"
