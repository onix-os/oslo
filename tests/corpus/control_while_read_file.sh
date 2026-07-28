# mode: posix
# The canonical line-reading loop. It must terminate at EOF.
printf 'one\ntwo\nthree\n' > lines.txt
while read -r line; do
    echo "[$line]"
done < lines.txt
echo done
