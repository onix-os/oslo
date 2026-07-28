# mode: posix
f() {
    echo "n=$#"
    echo "first=$1"
    echo "all=$*"
}
f one "two three"
f
