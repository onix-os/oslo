# mode: posix
# POSIX puts a loop's condition inside the loop, so `break` there ends the loop like any other.
# It used to unwind straight past it, taking the rest of the enclosing function with it — which
# is how a `while case ... esac; do shift; done` option parser silently skipped everything after
# it. modernish writes all of its option parsing that way.
while case x in *) break ;; esac
do
    echo body
done
echo after-while

until case x in *) break ;; esac
do
    echo body
done
echo after-until

while break; do echo body; done
echo after-plain

parse() {
    while case ${1-} in
        (-a) echo "saw -a" ;;
        (-b) echo "saw -b" ;;
        (*) break ;;
        esac
    do
        shift
    done
    echo "left=$#"
}
parse -a -b rest1 rest2

# The depth argument still counts outward from the loop the condition belongs to.
for i in 1 2 3; do
    while break 2; do :; done
    echo "unreachable-$i"
done
echo after-break2

# And an ordinary loop is unchanged.
i=0
while [ "$i" -lt 3 ]; do i=$((i + 1)); done
echo "i=$i"

i=0
while [ "$i" -lt 5 ]; do
    i=$((i + 1))
    [ "$i" = 2 ] && continue
    echo "n=$i"
done
