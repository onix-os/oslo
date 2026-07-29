# mode: posix
# The POSIX test grammar: connectives, their precedence, and parentheses.
[ x -a y ]; echo "$?"
[ "" -a x ]; echo "$?"
[ "" -o x ]; echo "$?"
[ x -a "" -o y ]; echo "$?"
[ -n x -a -n y -a -n z ]; echo "$?"
[ "" -o "" -o x ]; echo "$?"
[ \( a = b \) -o \( c = c \) ]; echo "$?"
[ \( a = b \) -a \( c = c \) ]; echo "$?"
[ ! \( a = b \) ]; echo "$?"
[ \( a \) ]; echo "$?"
[ \( "" \) ]; echo "$?"
