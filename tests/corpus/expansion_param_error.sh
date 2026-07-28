# mode: posix
# ${x:?word} aborts a non-interactive shell.
unset v
echo "${v:?is unset}"
echo NOT_REACHED
