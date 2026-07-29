# mode: bash
mkdir -p a b
pushd a > /dev/null
pushd ../b > /dev/null
dirs -v
dirs -p
dirs -l
dirs +1
dirs -0
dirs -c
dirs
echo "$?"
