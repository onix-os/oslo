# mode: posix
mkdir -p real/sub
ln -s real link
cd link
pwd
echo "$PWD"
cd sub
pwd
cd ..
pwd
pwd -P
cd -P .
pwd
