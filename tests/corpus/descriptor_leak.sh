# mode: posix
# A program the shell runs must see 0, 1 and 2 and nothing else. The shell's private copies of its
# own stdin and stdout were made with a bare `dup` — no `FD_CLOEXEC`, and on the lowest free number,
# which is inside the 3..9 a script addresses — so every child inherited them.
echo one | cat | /bin/sh -c 'if [ -e /proc/self/fd/4 ]; then echo "fd4=leaked"; else echo "fd4=clean"; fi'
echo two | cat | /bin/sh -c 'if [ -e /proc/self/fd/5 ]; then echo "fd5=leaked"; else echo "fd5=clean"; fi'
captured=$(echo three | cat | /bin/sh -c 'if [ -e /proc/self/fd/4 ]; then echo leaked; else echo clean; fi')
echo "in_capture=$captured"
