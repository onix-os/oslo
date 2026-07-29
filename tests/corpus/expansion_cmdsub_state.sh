# mode: posix
# A command substitution and a pipeline stage are children of this shell, not new shells:
# both see its functions and positional parameters.
emit() { echo "emit $1"; }
set -- one two three
echo "$(emit "$2")"
echo "$(echo "count=$#")"
false
echo "$(echo "status=$?")"
emit x | tr '[:lower:]' '[:upper:]'
echo "$#" | cat
secret=classified
echo "leak=$(env | grep -c '^secret=')"
