# mode: bash
# Quoting decides whether a brace is syntax, and a group produces whole words, not fields.
v=x
echo {$v,y}
echo x{$v,y}
echo "{a,b}"
echo '{a,b}'
echo {a\,b}
echo \{a,b\}
echo "{a,b}"{1,2}
set -- {1..3}
echo $#
IFS=,
w={a,b}
echo "$w"
