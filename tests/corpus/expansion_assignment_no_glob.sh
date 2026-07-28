# mode: posix
# x=*.txt stores the pattern, it does not expand it.
touch a.txt
touch b.txt
x=*.txt
echo "$x"
echo $x
