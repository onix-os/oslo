# mode: bash
unalias -a
alias q='echo hi'
unalias no_such_alias 2>/dev/null
echo "missing=$?"
unalias q
echo "removed=$?"
alias
echo "empty=$?"
