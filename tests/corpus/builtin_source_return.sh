# mode: posix
# `return` inside a sourced file ends the file, not the shell, and supplies its status.
printf 'echo one\nreturn 3\necho two\n' > lib.sh
. ./lib.sh
echo "rc=$?"
echo still-running
