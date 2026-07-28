# mode: posix
mkdir -p x/y
cd x/y
[ "$(pwd)" = "$PWD" ] && echo pwd_matches
case "$PWD" in */x/y) echo suffix_ok ;; *) echo suffix_bad ;; esac
