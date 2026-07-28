# mode: posix
# An assignment-only command takes the status of its last substitution.
x=$(exit 5)
echo "$?"
y=$(true)
echo "$?"
