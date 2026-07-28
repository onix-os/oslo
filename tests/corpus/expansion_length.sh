# mode: posix
# ${#name} and the positional-count forms.
s=hello
echo "${#s}"
empty=
echo "${#empty}"
set -- a bb ccc
echo "${#}"
echo "$#"
echo "${#1}"
