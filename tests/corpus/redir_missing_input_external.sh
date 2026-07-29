# mode: posix
# The external path already continued past a failed redirection; it is here so the four command
# kinds are pinned to the same status by the same suite.
cat < /nonexistent-file-xyz
echo "status=$?"
echo CONTINUE
