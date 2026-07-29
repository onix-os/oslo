# mode: posix
# "$*" on an assignment RHS is joined with IFS, not split into fields.
IFS=:
set -- a b c
x=$*
echo "$x"
IFS=' '
z=$*
echo "$z"
IFS=:
v=one:two:three
echo "$v"
set -- $v
echo "$#"
