# mode: posix
# IFS whitespace collapses and is stripped at both ends; a non-whitespace IFS character
# delimits exactly one field, and any whitespace around it is absorbed into that delimiter.
IFS=' :'
v='a : b'
set -- $v
echo "$#"
printf '[%s]\n' "$@"
v='a: :b'
set -- $v
echo "$#"
printf '[%s]\n' "$@"
v='  a  b  '
set -- $v
echo "$#"
printf '[%s]\n' "$@"
v='  :  '
set -- $v
echo "$#"
printf '[%s]\n' "$@"
