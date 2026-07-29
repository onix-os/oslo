# mode: posix
# Quoting is per character, not per word: only the unquoted metacharacters glob.
touch ab ac
p=a
echo "$p"b
echo "$p"?
echo "$p"'?'
echo $p*
echo "a"[bc]
echo "a[bc]"
echo 'a'*
