# mode: posix
set -- one "two three" four
echo "$#"
printf '[%s]\n' "$@"
set --
echo "$#"
