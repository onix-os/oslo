# mode: posix
set -- -a -b value -- rest
while getopts "ab:" opt; do
    case "$opt" in
        a) echo "flag_a" ;;
        b) echo "opt_b=$OPTARG" ;;
    esac
done
shift $((OPTIND - 1))
echo "remaining=$*"
