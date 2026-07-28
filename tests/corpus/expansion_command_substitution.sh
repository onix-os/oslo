# mode: posix
x=$(echo hi)
echo "$x"
echo "$(echo a)$(echo b)"
echo "outer $(echo "inner $(echo deep)")"
