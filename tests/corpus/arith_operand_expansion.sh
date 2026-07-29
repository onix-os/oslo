# mode: posix
x=5
echo $(( $x + 1 ))
e="2 + 3"
echo $(( e ))
s=abcde
echo $(( ${#s} * 2 ))
i=2
echo $(( $(echo 4) / i ))
echo $(( $((1 + 2)) * 3 ))
echo $((~$i))
