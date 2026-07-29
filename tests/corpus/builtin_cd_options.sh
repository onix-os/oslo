# mode: posix
mkdir -p a b
cd -- a
pwd
cd -L ..
cd a b
echo "$?"
pwd
cd -x
echo "$?"
cd -P b
pwd
