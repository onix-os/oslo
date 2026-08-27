# mode: posix
# A file the kernel will not exec — no `#!` line — is run by the shell itself. POSIX requires it and
# bash, dash and zsh all do it. oslo did it for a bare `./script` but not for `command` or `exec`, so
# one shell gave three answers: `command ./x` said "cannot execute" and `exec ./x` ended the shell
# with "Exec format error".
printf 'echo ran "$@"\necho "zero=${0##*/}"\n' > noshebang.sh
chmod +x noshebang.sh

./noshebang.sh direct
command ./noshebang.sh viacommand
( exec ./noshebang.sh viaexec )
echo "shell_survived=$?"
