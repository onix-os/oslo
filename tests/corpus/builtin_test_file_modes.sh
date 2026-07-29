# mode: posix
# The file predicates that used to be unconditionally false in `test`.
touch f
chmod 755 f
[ -x f ] && echo executable
chmod 644 f
[ -x f ] || echo not_executable
[ -w f ] && echo writable
[ -r f ] && echo readable
ln -s f link
[ -L link ] && echo symlink
[ -h link ] && echo symlink_h
[ -f link ] && echo follows_link
[ -O f ] && echo owned_by_me
[ -G f ] && echo owned_by_my_group
[ f -ef link ] && echo same_inode
[ f -ef . ] || echo different_inode
[ -p f ] || echo not_fifo
[ -S f ] || echo not_socket
[ -b f ] || echo not_block
[ -c f ] || echo not_char
[ -t 1 ] || echo not_a_tty
