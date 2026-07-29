# mode: posix
# Same rule for a compound command: status 1, body not run, script continues.
{ echo body; } < /nonexistent-file-xyz
echo "status=$?"
echo CONTINUE
