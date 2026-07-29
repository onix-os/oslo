# mode: bash
# The optional operand of ^ and , selects which characters are eligible.
v=hello
echo "${v^^[aeiou]}"
echo "${v^l}"
echo "${v^h}"
u=HELLO
echo "${u,,[AEIOU]}"
echo "${u,H}"
echo "[${empty^^}]"
