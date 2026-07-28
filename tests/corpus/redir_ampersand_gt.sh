# mode: bash
{ echo out; echo err >&2; } &> all.txt
sort all.txt
