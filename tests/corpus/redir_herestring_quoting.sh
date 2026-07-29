# mode: bash
# R11.B3. A here-string's word gets ordinary quote removal, not a strip of one outer pair.
# Stripping the first and last character was right only for a word that is entirely quoted; it
# left inner quotes in place and, worse, ate the first and last character of anything that merely
# started and ended with the same quote character.
cat <<< a"b"c
cat <<< "a"b"c"
cat <<< 'x'y'z'
v=VAL
cat <<< 'literal $v'
cat <<< end"$v"
# A quoted here-string is one field even when its value has blanks in it.
s='two  spaces'
cat <<< "$s"
