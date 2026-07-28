# mode: posix
# A final line with no newline is still read, and read then reports failure.
printf 'a\nb' > lines.txt
while read -r line; do
    echo "[$line]"
done < lines.txt
echo done
