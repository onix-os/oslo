# mode: posix
# -a / -o must actually combine, not default to true.
[ -f /nonexistent-a -a -f /nonexistent-b ]
echo "$?"
[ -z "" -o -z x ]
echo "$?"
[ -n x -a -n y ]
echo "$?"
[ -n "" -o -n "" ]
echo "$?"
