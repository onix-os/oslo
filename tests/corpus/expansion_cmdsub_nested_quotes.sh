# mode: posix
# The scanner must track quotes, so a ) inside quotes does not end the substitution.
echo "$(echo "a)b")"
echo "$(echo '(paren)')"
