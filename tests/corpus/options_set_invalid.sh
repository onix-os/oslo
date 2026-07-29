# mode: posix
(set -z) 2>/dev/null
echo "letter=$?"
(set -o nosuchoption) 2>/dev/null
echo "name=$?"
echo done
