# mode: bash
f() { echo fn; }
export -f f
echo "func=$?"
export -f no_such_function 2>/dev/null
echo "missing=$?"
