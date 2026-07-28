# mode: posix
# "$@" is one field per positional, whatever they contain.
set -- "a b" c ""
printf '[%s]\n' "$@"
echo "$#"
set --
printf 'count=%s\n' $#
for a in "$@"; do echo "iter:$a"; done
echo done
