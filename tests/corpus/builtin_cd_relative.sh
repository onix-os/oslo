# mode: posix
base=$(pwd)
mkdir -p a/b
cd a/b
pwd
cd ..
pwd
cd "$base"
[ "$PWD" = "$base" ] && echo back_ok
