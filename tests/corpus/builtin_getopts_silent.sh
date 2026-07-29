# mode: posix
set -- -z -b
while getopts ":ab:" o; do
    echo "o=$o arg=${OPTARG-unset}"
done
echo "end $OPTIND"
