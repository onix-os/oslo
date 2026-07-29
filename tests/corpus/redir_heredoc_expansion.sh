# mode: posix
# R11.B2. An unquoted here-document body is expanded before the command reads it, and the live
# set is exactly the one double quotes have: parameters, `$(…)`, backticks, arithmetic, and a
# backslash before `$`, a backtick or another backslash. Everything else — quotes, `*`, `~` —
# is ordinary text, which is why the body needs its own scanner rather than the word scanner.
v=VAL
set -- one two
cat <<EOF
plain $v and ${v}s
sub $(echo subbed) and \`echo tick\`
arith $((2 * 3))
escaped \$v \\ \`
positional $1 $#
quotes 'single' "double" and it's fine
glob * ? [abc]
tilde ~ and ~root
blank line follows

trailing
EOF
# Field splitting must not run: a body is one document, and IFS has no say in it.
IFS=:
p=a:b:c
cat <<EOF
$p
EOF
