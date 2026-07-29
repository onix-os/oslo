# mode: posix
# The condition of `if`/`elif` is exempt from errexit: asking a question is not failing at it.
set -e
if false; then echo THEN; else echo else; fi
if false; then echo THEN; elif false; then echo ELIF; else echo else2; fi
if [ 1 -eq 2 ]; then echo NO; fi
echo survived
