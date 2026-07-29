# mode: posix
# A redirection on a compound command is opened once and stays open across every iteration, so
# `>` does not truncate between them and `>>` starts from what is already there.
for i in 1 2 3; do echo "line$i"; done > out.txt
wc -l < out.txt
for i in 4 5; do echo "line$i"; done >> out.txt
cat out.txt
