# mode: posix
# A redirection that cannot be opened fails the call; it must not run the body and must not
# take the script down with it.
f() { echo body; }
f < /nonexistent-file-xyz
echo "status=$?"
echo CONTINUE
