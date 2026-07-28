# mode: posix
# A command substitution runs in a child that must still know the shell's functions.
helper() { echo from_function; }
x=$(helper)
echo "$x"
