# mode: posix
set -- outer1 outer2
f() { echo "inner=$1"; }
f inner1
echo "outer=$1"
echo "count=$#"
