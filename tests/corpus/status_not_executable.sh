# mode: posix
mkdir adir
./adir 2>/dev/null
echo "$?"
touch plain
chmod 644 plain
./plain 2>/dev/null
echo "$?"
