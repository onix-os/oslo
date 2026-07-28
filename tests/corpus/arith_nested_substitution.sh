# mode: posix
printf 'x\ny\n' > f.txt
echo $(( $(wc -l < f.txt) * 2 ))
s=abcde
echo $(( ${#s} + 1 ))
