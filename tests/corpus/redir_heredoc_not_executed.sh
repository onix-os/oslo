# mode: posix
# A heredoc body is data. It must never be run as commands.
cat <<EOF
touch heredoc_executed_marker
EOF
[ -e heredoc_executed_marker ] && echo EXECUTED || echo SAFE
