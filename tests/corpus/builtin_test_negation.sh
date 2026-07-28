# mode: posix
[ ! -f /nonexistent-file ]
echo "$?"
[ ! -f /etc/hostname ]
echo "$?"
[ ! a = a ]
echo "$?"
[ ! a = b ]
echo "$?"
