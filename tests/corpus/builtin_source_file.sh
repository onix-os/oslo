# mode: posix
printf 'sourced_var=hello\nsourced_fn() { echo from_sourced; }\n' > lib.sh
. ./lib.sh
echo "$sourced_var"
sourced_fn
