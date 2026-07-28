# mode: bash
# The same heredoc, in a script the adapter rejects. The body is still data.
x=5
((x++))
cat <<EOF
touch heredoc_executed_marker
EOF
[ -e heredoc_executed_marker ] && echo EXECUTED || echo SAFE
