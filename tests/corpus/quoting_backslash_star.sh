# mode: posix
# A backslash-escaped * is a literal, even though a bare * would glob.
touch xx
touch yy
echo \*
echo \*\*
