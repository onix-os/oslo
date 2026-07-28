# mode: posix
plain=1
export exported=2
sh -c 'echo "plain=${plain:-unset} exported=${exported:-unset}"'
export plain
sh -c 'echo "plain=${plain:-unset}"'
