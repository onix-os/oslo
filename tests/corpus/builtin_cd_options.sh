# mode: posix
mkdir -p a b
cd -- a
pwd
cd -L ..
pwd
cd -x
echo "$?"
cd -P b
pwd
