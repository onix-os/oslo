# mode: posix
# An expression test cannot parse must be an error, not "true".
[ a -qq b ]
echo "$?"
