# mode: posix
# A background job is this shell in another process too: it keeps functions and positionals,
# and a private variable stays private in it.
worker() { echo "worker $1 $#"; }
set -- bgarg
secret=classified
worker "$1" > out.txt &
wait
cat out.txt
{ env | grep -c '^secret=' > leak.txt; } &
wait
cat leak.txt
