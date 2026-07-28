# mode: posix
# ${x:=word} assigns and the assignment persists.
unset x
echo "${x:=assigned}"
echo "$x"
y=keep
echo "${y:=other}"
