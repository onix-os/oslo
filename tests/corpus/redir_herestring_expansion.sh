# mode: bash
# A here-string's word goes through the ordinary word expansions. rush lowers `<<< word` to a
# heredoc whose body is the *source text* with quotes stripped, so nothing in it expands — the
# same defect the unquoted heredoc body has, on a construct the size test also uses.
v=world
cat <<< "hello $v"
cat <<< "$(echo from-substitution)"
n=3
cat <<< "count=$n"
