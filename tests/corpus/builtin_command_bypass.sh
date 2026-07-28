# mode: posix
echo() { printf 'shadowed\n'; }
echo hi
command echo hi
