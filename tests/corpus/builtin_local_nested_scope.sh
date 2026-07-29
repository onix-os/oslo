# mode: posix
v=global
inner() {
    local v=inner
    echo "inner=$v"
}
outer() {
    local v=outer
    inner
    echo "outer=$v"
}
outer
echo "global=$v"
