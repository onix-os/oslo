# mode: posix
# A backslash-escaped space joins one field.
printf '[%s]\n' a\ b
printf '[%s]\n' a b
