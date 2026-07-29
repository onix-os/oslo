# mode: bash
printf 'a\nb\nc\n' > lines.txt

mapfile -t arr < lines.txt
echo "n=${#arr[@]} [${arr[0]}][${arr[1]}][${arr[2]}]"

# Without -t the delimiter is part of the record.
mapfile keep < lines.txt
printf '<%s>' "${keep[@]}"; echo

# -s skips records, -n stops after that many.
mapfile -t -s 1 -n 1 win < lines.txt
echo "win=${#win[@]} [${win[0]}]"

# -O decides where the first element lands; lower indices stay empty.
mapfile -t -O 2 off < lines.txt
echo "off=[${off[2]}][${off[3]}][${off[0]}]"

# `readarray` is the same builtin, and MAPFILE is the array when none is named.
readarray -t alias < lines.txt
echo "alias=[${alias[1]}]"
mapfile -t < lines.txt
echo "default=[${MAPFILE[2]}] n=${#MAPFILE[@]}"

# The reason this is not a `while read` loop: a last line with no newline is data.
printf 'x\ny' > nonl.txt
mapfile -t last < nonl.txt
echo "last=${#last[@]} [${last[1]}]"

# -d '' is the NUL delimiter `find -print0` produces.
printf 'p\0q\0' > nul.txt
mapfile -d '' -t z < nul.txt
echo "nul=${#z[@]} [${z[0]}][${z[1]}]"

# -u reads a descriptor other than stdin.
exec 7< lines.txt
mapfile -t -u 7 fd
echo "fd=${#fd[@]} [${fd[0]}]"
exec 7<&-

mapfile -Z bad 2>/dev/null; echo "badflag=$?"
mapfile -t 1bad < lines.txt 2>/dev/null; echo "badname=$?"
mapfile -t a b < lines.txt; echo "extra=$? [${a[0]}]"
