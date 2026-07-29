# mode: posix
# An assignment-only command still performs its redirection: the assignment happens, the failed
# open is reported, and the status is the redirection's, not the assignment's.
x=1 < /nonexistent-file-xyz
echo "status=$? x=$x"
echo CONTINUE
