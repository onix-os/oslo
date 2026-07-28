# mode: posix
{ echo a; echo b; } | tr 'ab' 'AB'
{ echo x; } > grouped.txt
cat grouped.txt
