# mode: posix
# A failed redirection must not abort the script.
cat < /nonexistent-file-xyz
echo "status=$?"
echo CONTINUE
