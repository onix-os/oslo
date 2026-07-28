# mode: posix
true; echo "$?"
false; echo "$?"
sh -c 'exit 42'; echo "$?"
