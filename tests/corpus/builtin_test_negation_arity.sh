# mode: posix
# `!` has to negate at every operand count, and be data when it lands in an operand slot.
[ ! ]; echo "$?"
[ ! "" ]; echo "$?"
[ ! x ]; echo "$?"
[ ! -z "" ]; echo "$?"
[ ! -n "" ]; echo "$?"
[ ! ! a = a ]; echo "$?"
[ ! \( -f /nonexistent-rush-file \) ]; echo "$?"
[ -f ! ]; echo "$?"
