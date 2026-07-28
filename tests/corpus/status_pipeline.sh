# mode: posix
false | true; echo "$?"
true | false; echo "$?"
false | false; echo "$?"
echo x | sh -c 'exit 4'; echo "$?"
