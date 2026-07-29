# mode: bash
# Substring operands are arithmetic and may nest, and the `:-` family still wins over a bare `:`.
v=abcdefgh
i=1
n=3
echo "${v:i+1:n-1}"
echo "${v:${i}:${n}}"
echo "${v:$((i*2))}"
# `:-` is a default, not an offset of -1 — that is why a negative offset needs a space or parens.
echo "[${v:-1}]"
echo "[${nosuch:-1}]"
echo "${v:(-2)}"
echo "${v:1:-2}"
# A window that starts past the end is empty, not an error.
echo "[${v:99}]"
echo "[${v:2:0}]"
