# mode: posix
# The four combinations of $@ / $* with and without quotes.
set -- "a b" c
printf '[%s]\n' $@
echo ---
printf '[%s]\n' $*
echo ---
printf '[%s]\n' "$@"
echo ---
printf '[%s]\n' "$*"
