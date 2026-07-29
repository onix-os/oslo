# mode: bash
# One line per escape family, plus the two rules that are easy to get wrong: an unknown escape
# keeps its backslash, and $'...' is inert inside double quotes.
printf '%s\n' $'tab:a\tb' | tr '\t' '|'
printf '%s\n' $'oct:\101 hex:\x41'
printf '%s\n' $'quote:\' backslash:\\'
printf '%s\n' $'unknown:\d'
printf '%s\n' "$'a\tb'"
x=$'a\nb'
printf '%s\n' "$x"
