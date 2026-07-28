# mode: posix
# Literal text is never field-split, whatever IFS says.
IFS=:
echo a:b:c
printf '[%s]\n' a:b:c
