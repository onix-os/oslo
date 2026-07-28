# mode: posix
read -r x < /nonexistent-file-xyz
echo "status=$?"
echo CONTINUE
