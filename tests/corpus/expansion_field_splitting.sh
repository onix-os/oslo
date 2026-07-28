# mode: posix
# Unquoted expansion output is split on IFS; quoted output is not.
v="a b c"
set -- $v
echo "$#"
set -- "$v"
echo "$#"
IFS=:
w=x:y:z
set -- $w
echo "$#"
printf '%s\n' "$@"
