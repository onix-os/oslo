# mode: posix
# A brace payload may itself contain a brace expansion.
unset x
y=inner
echo "${x:-${y}}"
echo "${x:-${y}-suffix}"
