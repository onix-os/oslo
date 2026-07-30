# mode: posix
# A comment inside `$( … )` whose `#` is preceded by an *odd* number of blanks is not recognised
# as a comment by brush-parser 0.4.0, so an unbalanced quote in it opens one that never closes and
# the whole file is a syntax error. Two blanks work, none work, one does not.
#
# Minimal form, 7 bytes: `$( #'` + newline + `)`. bash accepts every shape below. Note the comment
# needs an *odd* number of quotes: `it's the shell's` has two and they pair up harmlessly.
x=$(
	# store this subshell's pid
	echo body
)
echo "one-tab=[$x]"

y=$(
  # store this subshell's pid
  echo body2
)
echo "two-space=[$y]"

z=$( # store this subshell's pid
    echo body3
)
echo "opener=[$z]"
