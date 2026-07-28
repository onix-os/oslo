# mode: posix
v=global
f() {
    local v=local
    echo "in=$v"
}
f
echo "out=$v"
