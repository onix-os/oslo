# mode: posix
# `${1+"$@"}` is the pre-POSIX way to forward an argument list, and it is still what a lot of
# portable shell — modernish's own diagnostics included — is written with. Expanding the payload
# to a single string joined the arguments on a space and then re-split that join on IFS, so
# `printf '%s\n' ${1+"$@"}` printed one line per word rather than one per argument.
set -- "a b" "c d"
printf '[%s]\n' ${1+"$@"}
printf '[%s]\n' "${1+"$@"}"
printf '[%s]\n' ${1-"$@"}

# With no arguments the payload contributes nothing at all, not one empty field.
set --
printf '[%s]\n' ${1+"$@"}
echo "after=$#"

# The payload's own literal text is part of an expansion result, so it splits on IFS...
unset v
IFS=:
printf '[%s]\n' ${v-a:b}
IFS=' '
# ...but its quoted parts do not.
printf '[%s]\n' ${v-"a b"}
printf '[%s]\n' "${v-a b}"
printf '[%s]\n' ${v+never}

# `:=` still assigns one string, whatever the expansion splits into.
unset v
printf '[%s]\n' ${v:=a b}
echo "v=[$v]"

# The alternate branch of a set variable, and the default branch of an unset one.
w=x
printf '[%s]\n' ${w+y z}
printf '[%s]\n' ${w-y z}
