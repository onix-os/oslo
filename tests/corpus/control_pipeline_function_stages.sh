# mode: posix
# Functions are ordinary pipeline stages: they read the pipe on stdin and write to the next one.
up() { tr a-z A-Z; }
gen() { echo alpha; echo beta; }
gen | up | tail -1
gen | while read -r w; do echo "got:$w"; done | wc -l
