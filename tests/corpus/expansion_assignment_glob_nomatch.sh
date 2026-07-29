# mode: posix
# Neither a matching nor a non-matching pattern expands on an assignment RHS.
touch one.txt two.txt
p=*.txt
q=*.nope
echo "$p"
echo "$q"
printf '%s\n' $p
printf '%s\n' $q
