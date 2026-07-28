# mode: posix
[ 3 -gt 2 ]; echo "$?"
[ 3 -lt 2 ]; echo "$?"
[ 3 -eq 3 ]; echo "$?"
[ 3 -ne 3 ]; echo "$?"
[ 2 -le 2 ]; echo "$?"
[ 2 -ge 3 ]; echo "$?"
