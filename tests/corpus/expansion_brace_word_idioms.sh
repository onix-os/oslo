# mode: bash
# The idioms brace expansion exists for: one word in the script, several *words* out.
#
# This case is the guard on that property, and on where it does and does not apply. Brace
# expansion is the only expansion that turns one word into several words rather than one word into
# several fields, so nothing later can recover what is lost if it stops happening —
# `mkdir -p build/{bin,lib}` would quietly make a single directory named `build/{bin,lib}` and
# every path built from it afterwards would miss.
mkdir -p build/{bin,lib}
ls build
mkdir -p out/{a,b}/{x,y}
ls out/a out/b
touch file{1,2,3}.txt
ls file*.txt
for d in build/{bin,lib}; do printf '%s\n' "$d"; done
set -- pkg/{one,two}
echo $#
printf '%s\n' "$@"
# A group in the command-name position is a group like any other.
{echo,printf} name-position
# An array literal is a word list too.
a=(x{1,2} {3..4})
printf '%s\n' "${a[@]}"
declare -a b="(y{1,2})"
printf '%s\n' "${b[@]}"
# …and these positions are not word lists, so a group in them is plain text.
w={a,b}
echo "$w"
w2=x{1..3}
echo "$w2"
case '{a,b}' in {a,b}) echo pattern-is-literal;; *) echo pattern-expanded;; esac
[[ '{a,b}' == {a,b} ]] && echo test-operand-is-literal
# Quoting, escaping and non-groups leave the text exactly as it was typed.
echo "{a,b}" '{a,b}' {a\,b} \{a,b\} {a}
echo a{b}c {} {a}{b,c}
