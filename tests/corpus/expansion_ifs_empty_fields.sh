# mode: posix
# A non-whitespace IFS character delimits exactly one field, so a::b is three fields.
IFS=:
v=a::b
set -- $v
echo "$#"
printf '[%s]\n' "$@"
v=:a:
set -- $v
echo "$#"
