# mode: posix
ls /nonexistent-directory-xyz 2> err.txt
echo "status=$?"
[ -s err.txt ] && echo has_stderr
