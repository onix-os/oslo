# mode: bash
echo() { printf 'shadowed\n'; }
echo hi
builtin echo forced
builtin ls
builtin echo "rc=$?"
