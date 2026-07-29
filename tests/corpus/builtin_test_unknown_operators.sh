# mode: posix
# An operator test does not know is a syntax error; answering "false" would be indistinguishable
# from a real negative result.
[ -q file ]; echo "$?"
[ a -qq b ]; echo "$?"
[ a -lte b ]; echo "$?"
[ -f /etc -eq 1 ]; echo "$?"
