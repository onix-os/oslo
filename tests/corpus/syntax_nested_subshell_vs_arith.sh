# mode: bash
# A subshell opening with another subshell is not an arithmetic command. Only adjacent
# parens are `((` — with anything between them, including a space, these are two groups.
( ( echo spaced ) )
( (echo inner-tight) )
( ( ( echo three-deep ) ) )
( ( exit 3 ) ); echo "status=$?"
echo x | ( ( cat ) )

# And the adjacent spelling still is arithmetic, in every position it appears in.
(( 1 + 1 )); echo "arith=$?"
(( 0 )); echo "false=$?"
n=5; (( n++ )); echo "n=$n"
echo "sub=$(( 6 * 7 ))"
for (( i = 0; i < 3; i++ )); do printf '%s' "$i"; done; echo
if (( 2 > 1 )); then echo "cond"; fi
