# mode: posix
readonly RP=value
readonly -p | grep -c '^readonly RP='
readonly -p > /dev/null
echo "after=$RP"
