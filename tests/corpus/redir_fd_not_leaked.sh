# mode: posix
# Children must not inherit the shell's saved descriptors.
echo x > f.txt
sh -c 'ls /proc/self/fd 2>/dev/null | sort | tr "\n" " "; echo' < f.txt
