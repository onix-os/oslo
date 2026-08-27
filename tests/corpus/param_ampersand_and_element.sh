# mode: bash
# `&` in a replacement stands for what matched, unless it was quoted. The escaped spelling already
# agreed, so only the match-reference form was wrong — silently.
v=abc
echo "all=[${v//?/[&]}]"
echo "one=[${v/b/[&]}]"
echo "escaped=[${v/b/[\&]}]"
echo "singled=[${v/b/'&'}]"
echo "doubled=[${v/b/"&"}]"
r="&"
echo "unquoted_var=[${v/b/$r}]"
echo "quoted_var=[${v/b/"$r"}]"
echo "twice=[${v//b/&&}]"
echo "anchored=[${v/#a/[&]}] [${v/%c/[&]}]"
declare -a pair=(ab cb)
echo "elementwise=[${pair[@]/b/[&]}]"

# A scalar is an array of one: `${v[0]}` is the value, `${#v[0]}` its length.
s=x
echo "elem=[${s[0]}] len=[${#s[0]}] default=[${s[0]:-d}]"
echo "past_end=[${s[1]}]"
declare -a real=(p q)
echo "real=[${real[0]}][${real[1]}][${real[2]}]"
