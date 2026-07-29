# mode: posix
# A function outranks every non-special builtin, including the ones a shell implements
# natively. Wrapping `echo`, `cd` or `test` is how a script overrides behaviour it does not
# control; a shell that resolves the builtin first runs the original and ignores the wrapper.
echo() { printf 'wrapped echo: %s\n' "$1"; }
echo hi

cd() { printf 'wrapped cd: %s\n' "$1"; }
cd /
pwd

test() { printf 'wrapped test\n'; return 3; }
test -f /etc/hosts
printf 'status %s\n' "$?"
