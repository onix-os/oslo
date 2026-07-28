# mode: posix
# A background command's own status is 0 and its notice never lands on stdout.
x=$(sleep 0 & echo captured)
echo "[$x]"
sleep 0 &
echo "bg=$?"
wait
