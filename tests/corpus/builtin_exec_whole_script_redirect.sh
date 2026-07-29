# mode: posix
( exec > out.txt
  echo captured
  echo also captured )
cat out.txt
