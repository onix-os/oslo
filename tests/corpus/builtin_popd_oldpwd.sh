# mode: bash
mkdir -p a b
pushd a > /dev/null
pushd ../b > /dev/null
popd > /dev/null
cd -
pwd
echo "$OLDPWD"
popd > /dev/null
popd
echo "$?"
