# mode: posix
# "$@" splices into the text around it: the first positional joins what precedes, the last joins
# what follows, and with no positionals the two neighbours simply meet.
set -- 1 2 3
printf '[%s]\n' x"$@"y
printf '[%s]\n' "pre $* post"
set --
printf '[%s]\n' x"$@"y
echo done
