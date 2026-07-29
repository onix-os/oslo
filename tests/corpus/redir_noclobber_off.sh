# mode: posix
# Without -C, `>` truncates whatever is there. This is the "option off" half of R6.3.
echo first > out.txt
echo second > out.txt
echo "status=$?"
cat out.txt
