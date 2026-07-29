# mode: bash
mkdir -p a b c
pushd a
pushd ../b
pushd ../c
dirs
pushd +1
pwd
pushd -1
pwd
pushd
pwd
pushd +9
echo "$?"
popd
dirs
