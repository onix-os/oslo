# mode: posix
# `exec` redirects the shell it runs in — a subshell's, here — so the parent's stdout is intact
# once the subshell is gone.
( exec 1>inner.txt; echo hidden )
echo visible
cat inner.txt
