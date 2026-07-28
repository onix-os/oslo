# mode: posix
countdown() {
    [ "$1" -le 0 ] && return 0
    echo "$1"
    countdown $(($1 - 1))
}
countdown 3
