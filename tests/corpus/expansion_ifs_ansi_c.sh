# mode: bash
# The safe-filename idiom: IFS set to a real newline, not the three characters $ \ n.
IFS=$'\n'
v='a b
c d'
set -- $v
echo "$#"
printf '[%s]\n' "$@"
