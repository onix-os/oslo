# mode: posix
# Symbolic modes are applied to the permissions the mask keeps, not to the mask.
umask 022
umask u=rwx,g=,o=
umask
umask -S
umask a-w
umask
umask u+w,go-rx
umask
umask -p
umask abc 2>/dev/null
echo "bad=$?"
umask
