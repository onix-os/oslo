# mode: posix
# noclobber refuses to *overwrite*; it never refuses to create.
set -C
echo made > fresh.txt
echo "status=$?"
cat fresh.txt
