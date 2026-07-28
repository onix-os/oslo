# mode: posix
printf '[%s]\n' "" x ''
e=
printf '[%s]\n' "$e"
set -- "$e"
echo "$#"
