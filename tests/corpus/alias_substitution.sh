# mode: bash
# Alias substitution happens on the source text, before parsing — an alias body is source, not a
# list of arguments. oslo used to substitute after parsing, which handles `alias ll='ls -la'` and
# nothing else; by then an alias that opens a construct has already been a syntax error.
#
# bash needs `expand_aliases` to do any of this in a script; oslo follows POSIX and always does.
shopt -s expand_aliases

alias e='echo one two'
e
alias p='echo prefix'
p suffix
alias q='echo "a  b"'
q

# An alias is not available on the line that defines it, in either shell.
alias late='echo LATE'; late 2>/dev/null || echo "not-yet"

# It contributes syntax: the body opens a loop the word it replaced could not.
alias forever='while :; do'
n=0
forever
    n=$((n + 1))
    [ "$n" -ge 3 ] && break
done
echo "n=$n"

# Chaining, and a self-reference that has to terminate.
alias inner='echo INNER'
alias outer='inner --flag'
outer
alias selfref='selfref -x'
selfref 2>/dev/null || echo "selfref ran once"

# A trailing blank makes the next word a candidate too.
alias run='run_it '
alias target='echo TARGET'
run_it() { echo "run_it $*"; }
run target

# Only in command position.
echo e
alias notarg='echo BAD'
echo notarg
for w in notarg; do echo "word=$w"; done
case notarg in notarg) echo "pattern kept" ;; esac

# Not inside arithmetic or a parameter expansion, which hold no commands.
alias n='echo BAD'
n=7
echo "arith=$((n + 1))"
echo "param=${n}"

# Quoted text and comments are not command words.
echo 'e'
echo "e"
# e
