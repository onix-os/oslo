# mode: posix
# Quoted and unquoted runs in one word: the unquoted * must still glob.
touch a1
touch a2
echo "a"*
echo "a*"
echo 'a'*
