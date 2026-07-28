# mode: posix
set -- a b c d
shift
echo "$*"
shift 2
echo "$* n=$#"
shift 5
echo "over=$?"
