# mode: posix
# A Unicode blank that is not a shell separator is an ordinary word character, exactly as it is
# in bash. oslo used to hang here at *parse* time: the token scanner stepped over a set of
# characters the word scanner refused to consume, so the lexer handed back empty words forever
# and the parser grew a Vec until the allocator aborted the process. A no-break space pasted out
# of a web page was enough. See fuzz/known/README.md.
echo ab
echo a b
echo ab
echo a b
echo "xy"
v=1 2
echo "[$v]"
for w in pq r s; do echo "<$w>"; done
echo 'qz'
