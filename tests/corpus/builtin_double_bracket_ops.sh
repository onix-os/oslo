# mode: bash
# `[[ ]]` shares the operator table with `[`, but keeps its own `==` pattern rule.
touch empty
printf 'x\n' > full
[[ abc == a* ]] && echo glob_match
[[ abc == "a*" ]] || echo quoted_is_literal
[[ abc == abc ]] && echo equal
[[ -d . ]] && echo dir
[[ -s full ]] && echo nonempty
[[ -s empty ]] || echo empty_file
[[ -x full ]] || echo not_executable
[[ 3 -gt 2 ]] && echo greater
[[ 2 -ge 2 ]] && echo at_least
