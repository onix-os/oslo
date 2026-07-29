# mode: posix
# Inside backticks only \` \\ and \$ are escapes; every other backslash is data, and the
# substitution stays live inside double quotes.
v=set
echo `echo \$v`
echo `echo '$v'`
echo "outer `echo nested` end"
echo `echo a``echo b`
echo "`echo one` `echo two`"
