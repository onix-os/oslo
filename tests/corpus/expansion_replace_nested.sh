# mode: bash
# ${v/pat/rep}: pat is a pattern, not a literal, and only the first / splits the operands.
v=a/b/c
echo "${v/\//_}"
echo "${v//\//_}"
p=one.two.three
echo "${p/./-}"
echo "${p//t*o/X}"
sep=.
echo "${p//$sep/ }"
q=abc
echo "[${q/b}]"
echo "[${q//}]"
echo "${q/#/X}"
echo "${q/%/Z}"
echo "${q/b//Y}"
echo "${q/x/Y}"
