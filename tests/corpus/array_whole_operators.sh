# mode: bash
# Operators applied to a *whole* array: a slice of the element list, and a pattern applied to
# every element. Both are real bash; rush rejects them loudly rather than answering something
# plausible, so this case is the record of what is still missing.
a=(alpha beta gamma)
echo "${a[@]:1}"
echo "${a[@]:1:1}"
echo "${a[@]#al}"
echo "${a[*]:2}"
