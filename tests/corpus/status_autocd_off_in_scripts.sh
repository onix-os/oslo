# mode: posix
# A command word that happens to name a directory is still a command. Changing directory
# instead makes every later relative path in the script resolve somewhere else, and says so
# with status 0.
mkdir sub
sub 2>/dev/null
printf 'bare %s\n' "$?"
./sub 2>/dev/null
printf 'dotslash %s\n' "$?"
./sub -x 2>/dev/null
printf 'with-args %s\n' "$?"
pwd
