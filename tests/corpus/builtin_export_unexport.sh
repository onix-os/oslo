# mode: bash
export XN=1
export -n XN
echo "value=$XN"
env | grep -c '^XN=' || true
sh -c 'echo "child=${XN:-unset}"'
