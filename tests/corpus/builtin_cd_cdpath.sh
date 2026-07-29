# mode: posix
mkdir -p top/inner
base=$(pwd)
CDPATH=$base/top
cd inner
pwd
cd "$base"
CDPATH=:$base/top
cd top
pwd
cd "$base"
CDPATH=/nonexistent-cdpath-xyz
cd top
pwd
cd "$base"
CDPATH=$base/top
cd ./inner
echo "$?"
pwd
