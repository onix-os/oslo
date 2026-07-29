# mode: posix
# Escaped whitespace and escaped IFS characters join one field; only expansion output splits.
IFS=:
printf '[%s]\n' a\ b\ c
printf '[%s]\n' a\:b
v=a:b
printf '[%s]\n' $v
printf '[%s]\n' "$v"
