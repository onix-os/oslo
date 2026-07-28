# mode: posix
outer() {
    inner() { echo "inner:$1"; }
    inner "$1"
    echo "outer:$1"
}
outer value
inner direct
