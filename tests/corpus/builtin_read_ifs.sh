# mode: posix
printf 'root:x:0:0:Root User:/root:/bin/sh\n' > pw
IFS=: read -r user pass uid gid rest < pw
echo "user=$user"
echo "uid=$uid"
echo "rest=$rest"
