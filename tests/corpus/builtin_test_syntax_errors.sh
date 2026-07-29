# mode: posix
# Deliberate garbage: each of these must be a diagnostic and exit 2, never a truth value.
[ a b ]; echo "$?"
[ a b c ]; echo "$?"
[ a b c d e ]; echo "$?"
[ a = ]; echo "$?"
[ x -a ]; echo "$?"
[ 1 -eq 1 -a ]; echo "$?"
[ \( \) ]; echo "$?"
[ \( a = a ]; echo "$?"
[ a = a \) ]; echo "$?"
