# mode: posix
set -- one two three
set -e
echo "kept=$#:$1"
set -f -- alpha beta
echo "replaced=$#:$1:$2"
set --
echo "cleared=$#"
set -- -x -u
echo "literal=$1:$2"
