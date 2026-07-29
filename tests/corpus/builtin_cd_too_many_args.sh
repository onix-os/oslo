# mode: posix
# needs-bash: 5.3
# Split out of builtin_cd_options.sh purely so that file keeps running against older oracles:
# this one assertion moved between releases, and the `cd --`/`-L`/`-x`/`-P` coverage next to it
# did not deserve to be skipped along with it.
#
# `cd` with two operands is a usage error, and bash 5.2 reported it as 1 while 5.3 reports 2.
# rush follows 5.3. Note this is *not* the same as an unknown option: `cd -x` was already 2 in
# 5.2, so only the operand-count path changed.
mkdir -p a b
cd a b
echo "$?"
pwd
