# mode: bash
# Quoting the right operand of `=~` turns off its special meaning, exactly as it does for `==`.
[[ abc =~ a.c ]]; echo "unquoted=$?"
[[ abc =~ "a.c" ]]; echo "quoted=$?"
[[ a.c =~ "a.c" ]]; echo "literal-hit=$?"

# Text that would not even compile as a regex is ordinary text once quoted.
[[ abc =~ '(' ]]; echo "quoted-paren=$?"
[[ 'a(c' =~ '(' ]]; echo "quoted-paren-hit=$?"

# The same distinction reaches through a variable: `$re` is a pattern, `"$re"` is not.
re='^[0-9]+$'
[[ 42 =~ $re ]]; echo "var=$?"
[[ 42 =~ "$re" ]]; echo "var-quoted=$?"

# `=~` is a `[[ ]]` operator. In `test`/`[` it is not an operator at all, and bash exits 2.
[ abc =~ b ] 2>/dev/null; echo "posix-test=$?"
