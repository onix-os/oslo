# mode: posix
# Redirecting onto descriptors that were never open still has to be undone. Nothing was there to
# save, so the only faithful restore is to close them again — otherwise 5, 6 and 7 stay open for
# the rest of the shell's life and every later child inherits them.
true 5>a.txt 6>b.txt 7>c.txt
sh -c 'ls /proc/self/fd 2>/dev/null | sort | tr "\n" " "; echo'
echo "files=$(ls a.txt b.txt c.txt | tr "\n" " ")"
