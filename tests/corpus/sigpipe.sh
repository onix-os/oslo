# mode: posix
# The Rust runtime ignores SIGPIPE before `main`, and a shell must not. With it ignored,
# `oslo -c 'while :; do echo x; done' | head -1` ran for ever: nothing told the loop its reader
# had gone. Children always got SIG_DFL back; this is the shell's own disposition.
i=0
while [ "$i" -lt 100000 ]; do
    echo line
    i=$((i + 1))
done | head -3
echo "pipeline=$?"

# A subshell writing into a closed pipe is the same story.
(
    n=0
    while [ "$n" -lt 100000 ]; do
        printf 'x\n'
        n=$((n + 1))
    done
) | head -2
echo "subshell=$?"

# And the disposition itself is observable: a shell that ignores SIGPIPE survives being sent one.
sh -c 'kill -s PIPE $$; echo SURVIVED' 2>/dev/null
echo "self=$?"
