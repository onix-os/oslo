# mode: posix
# Arithmetic comparisons are decimal-only; a non-numeric operand is an error, not a zero.
[ " 7 " -eq 7 ]; echo "$?"
[ +7 -eq 7 ]; echo "$?"
[ 010 -eq 10 ]; echo "$?"
[ -1 -lt 0 ]; echo "$?"
[ 3 -eq abc ]; echo "$?"
[ "" -eq 0 ]; echo "$?"
[ 0x10 -eq 16 ]; echo "$?"
# -a/-o do not short-circuit, so the far side's bad operand still reports.
[ 1 -eq 1 -o abc -eq 1 ]; echo "$?"
