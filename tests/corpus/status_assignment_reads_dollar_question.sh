# mode: posix
# An assignment's right-hand side is expanded before the assignment becomes a command of its
# own, so `$?` there is still the *previous* command's status — the idiom every error handler
# is written with. Afterwards the assignment reports its own status, 0 when nothing ran inside.
false
x=$?
echo "x=$x"
echo "after=$?"
true
y=$? z=9
echo "y=$y z=$z after=$?"
