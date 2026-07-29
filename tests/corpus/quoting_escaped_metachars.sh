# mode: posix
# A backslash makes a metacharacter literal even where a bare one would glob.
touch aq az
echo a\?
echo a\*
echo \[abc\]
echo a?
echo a\?b
