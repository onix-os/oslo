# mode: posix
# The if condition's status must not leak into the command's own status.
if true; then true; fi
echo "$?"
if false; then true; fi
echo "$?"
