# mode: posix
{ echo out; echo err >&2; } > all.txt 2>&1
sort all.txt
